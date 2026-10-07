//! Page builders: GitHub's pages (repository, file, lists, issue, pull
//! request, profile, search, home) as [`Page`]s, laid out the way GitHub
//! lays them out: boxes for file lists, READMEs, lists and comments, and a
//! sidebar when there's room. Every link is the github.com URL of what it
//! points at; `ghtui:` links are actions (star, load more, filter...).

use std::collections::HashMap;

use ghtui_api::browse::{
    Advisory, Blame, Blob, BranchInfo, CheckItem, CheckOutcome, Checks, Comment, CommitDetail,
    CommitInfo, Comparison, Contributions, DeploymentList, DiscussionDetail, DiscussionList,
    EntryKind, Gist, GistSummary, IssueDetail, IssueState, IssueSummary, Job, JobSummary,
    MilestoneDetail, MilestoneInfo, MilestoneList, PrActivity, Profile, Release, RepoOverview,
    RepoSort, RepoSummary, Results, RunSummary, SearchKind, SearchResults, TagInfo, TeamDetail,
    TeamSummary, TreeEntry, UserSummary, WikiPage, Workflow, WorkflowRun,
};
use ghtui_api::model::{
    ChecksState, Inbox, Label, Mergeable, PrDetail, PrRef, PrState, PrSummary, RepoId,
    ReviewDecision,
};
use ghtui_theme::{Bg, Syntax};

use crate::markdown::{self, LinkBase};
use crate::page::{ASIDE_GAP, Frame, Link, Page, PageLine, Role, Seg, Tone};
use crate::{Icons, cols, time};

// ---- URLs ----------------------------------------------------------------------

pub mod url {
    use super::*;

    pub const BASE: &str = "https://github.com";

    pub fn user(login: &str) -> String {
        format!("{BASE}/{login}")
    }
    pub fn repo(repo: &RepoId) -> String {
        format!("{BASE}/{repo}")
    }
    pub fn tree(repo: &RepoId, rev: &str, path: &str) -> String {
        let (rev, path) = (encode_path(rev), encode_path(path));
        if path.is_empty() {
            format!("{BASE}/{repo}/tree/{rev}")
        } else {
            format!("{BASE}/{repo}/tree/{rev}/{path}")
        }
    }
    pub fn blob(repo: &RepoId, rev: &str, path: &str) -> String {
        let (rev, path) = (encode_path(rev), encode_path(path));
        format!("{BASE}/{repo}/blob/{rev}/{path}")
    }
    pub fn issues(repo: &RepoId) -> String {
        format!("{BASE}/{repo}/issues")
    }
    pub fn pulls(repo: &RepoId) -> String {
        format!("{BASE}/{repo}/pulls")
    }
    pub fn issue(repo: &RepoId, number: u64) -> String {
        format!("{BASE}/{repo}/issues/{number}")
    }
    pub fn pull(pr: &PrRef) -> String {
        format!("{BASE}/{}/pull/{}", pr.repo, pr.number)
    }
    pub fn pull_tab(pr: &PrRef, tab: &str) -> String {
        format!("{}/{tab}", pull(pr))
    }
    pub fn search(kind: SearchKind, query: &str) -> String {
        let kind = match kind {
            SearchKind::Repos => "repositories",
            SearchKind::Issues => "issues",
            SearchKind::Pulls => "pullrequests",
            SearchKind::Users => "users",
            SearchKind::Discussions => "discussions",
            SearchKind::Commits => "commits",
            SearchKind::Code => "code",
        };
        format!("{BASE}/search?q={}&type={kind}", encode(query))
    }
    pub fn commit(repo: &RepoId, oid: &str) -> String {
        format!("{BASE}/{repo}/commit/{oid}")
    }
    pub fn compare(repo: &RepoId, from: &str, to: &str) -> String {
        format!("{}/compare/{from}...{to}", self::repo(repo))
    }
    pub fn release(repo: &RepoId, tag: &str) -> String {
        format!("{BASE}/{repo}/releases/tag/{}", encode_path(tag))
    }
    /// A revision's history, of `path` if it isn't empty.
    pub fn commits(repo: &RepoId, rev: &str, path: &str) -> String {
        let rev = encode_path(rev);
        if path.is_empty() {
            format!("{BASE}/{repo}/commits/{rev}")
        } else {
            format!("{BASE}/{repo}/commits/{rev}/{}", encode_path(path))
        }
    }

    /// A file path in a URL: `/` separates segments, everything else that
    /// isn't plainly safe (`#`, `?`, `%`, `\\`, spaces, non-ASCII) is
    /// escaped.
    pub fn encode_path(s: &str) -> String {
        escape(s, b"-_.~/", false)
    }

    /// Minimal query-string encoding.
    pub fn encode(s: &str) -> String {
        escape(s, b"-_.~:/@", true)
    }

    /// `s` with every byte but ASCII alphanumerics and `safe` escaped, and
    /// spaces as `+` when `plus`.
    fn escape(s: &str, safe: &[u8], plus: bool) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            if b.is_ascii_alphanumeric() || safe.contains(&b) {
                out.push(char::from(b));
            } else if b == b' ' && plus {
                out.push('+');
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        }
        out
    }
}

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

fn plural(n: u64, word: &str) -> String {
    format!("{} {word}{}", compact(n), if n == 1 { "" } else { "s" })
}

fn chip(text: impl Into<String>, bg: Bg) -> Seg {
    Seg::new(format!(" {} ", text.into()), Role::Chip(bg))
}

fn space() -> Seg {
    Seg::new(" ", Role::Body)
}

fn labels(segs: &mut Vec<Seg>, labels: &[Label]) {
    for l in labels {
        segs.push(space());
        segs.push(Seg::new(
            format!(" {} ", l.name),
            Role::Label(l.color.clone()),
        ));
    }
}

fn link_seg(page: &mut Page, text: impl Into<String>, url: impl Into<Link>, role: Role) -> Seg {
    let link = page.link(url);
    Seg::linked(text, role, link)
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
    for line in crate::page::wrap_segs(vec![Seg::new(text, Role::Body)], width.saturating_sub(4)) {
        let text: String = line.iter().map(|s| s.text.as_str()).collect();
        let pad = width.saturating_sub(crate::text::width(&text) + 2);
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

/// One row per item, ruled apart.
fn box_rows<T>(page: &mut Page, items: &[T], mut row: impl FnMut(&mut Page, &T)) {
    for (n, item) in items.iter().enumerate() {
        if n > 0 && !page.compact {
            page.box_rule();
        }
        row(page, item);
    }
}

/// A box of `items`, a row each, saying `empty` when there are none.
fn list_box<T>(
    page: &mut Page,
    title: Vec<Seg>,
    right: Vec<Seg>,
    items: &[T],
    empty: &str,
    row: impl FnMut(&mut Page, &T),
) {
    page.box_top(title, right);
    if items.is_empty() {
        empty_row(page, empty);
    }
    box_rows(page, items, row);
    page.box_bottom();
}

/// `prefix` then `segs` wrapped inside a box, continuation lines aligned
/// after the prefix.
fn hanging(page: &mut Page, prefix: Seg, segs: Vec<Seg>, frame: Frame) {
    let width = cols(crate::text::width(&prefix.text));
    let start = page.lines.len();
    page.wrapped(segs, width, frame);
    if let Some(first) = page.lines.get_mut(start) {
        first.indent = 0;
        first.segs.insert(0, prefix);
    }
}

fn issue_icon(icons: Icons, state: IssueState, is_pr: bool) -> (String, Role) {
    let nerd = icons.nerd_font;
    match (state, is_pr) {
        (IssueState::Open, false) => (if nerd { "\u{f41b}" } else { "◉" }.into(), Role::Success),
        (IssueState::Closed, false) => (if nerd { "\u{f41d}" } else { "✓" }.into(), Role::Accent),
        (IssueState::NotPlanned, _) => (if nerd { "\u{f468}" } else { "⊘" }.into(), Role::Meta),
        (IssueState::Open, true) => (icons.pr_state(PrState::Open).into(), Role::Success),
        (IssueState::Draft, _) => (icons.pr_state(PrState::Draft).into(), Role::Meta),
        (IssueState::Merged, _) => (icons.pr_state(PrState::Merged).into(), Role::Accent),
        (IssueState::Closed, true) => (icons.pr_state(PrState::Closed).into(), Role::Error),
    }
}

fn state_chip(state: IssueState, is_pr: bool, icons: Icons) -> Seg {
    let (icon, _) = issue_icon(icons, state, is_pr);
    let (text, bg) = match state {
        IssueState::Open => ("Open", Bg::SuccessContainer),
        IssueState::Draft => ("Draft", Bg::SecondaryContainer),
        IssueState::Merged => ("Merged", Bg::TertiaryContainer),
        IssueState::NotPlanned => ("Closed as not planned", Bg::SecondaryContainer),
        IssueState::Closed if is_pr => ("Closed", Bg::ErrorContainer),
        IssueState::Closed => ("Closed", Bg::TertiaryContainer),
    };
    chip(format!("{icon} {text}"), bg)
}

fn pr_state(s: PrState) -> IssueState {
    match s {
        PrState::Open => IssueState::Open,
        PrState::Draft => IssueState::Draft,
        PrState::Closed => IssueState::Closed,
        PrState::Merged => IssueState::Merged,
    }
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
    pub entries: Option<&'a [TreeEntry]>,
    pub commits: Option<&'a HashMap<String, CommitInfo>>,
}

/// Width of a sidebar for a page `total` columns wide, if there's room.
pub fn aside_width(total: u16) -> Option<u16> {
    (total >= 104).then_some(30)
}

/// The main column's width beside an aside of `aside` columns.
pub fn main_width(total: u16, aside: Option<u16>) -> u16 {
    match aside {
        Some(a) => total.saturating_sub(a + ASIDE_GAP),
        None => total,
    }
}

fn aside_heading(page: &mut Page, text: &str) {
    page.blank();
    page.line(vec![Seg::new(text, Role::Strong)]);
}

// ---- repository ---------------------------------------------------------------------

/// The repository's title: `owner / name  Public` with Star, Fork and Watch
/// buttons on the right, as on GitHub.
pub fn repo_title(page: &mut Page, repo: &RepoId, overview: Option<&RepoOverview>) {
    let owner = link_seg(page, repo.owner.clone(), url::user(&repo.owner), Role::Link);
    let name = link_seg(page, repo.name.clone(), url::repo(repo), Role::Title);
    let mut segs = vec![owner, Seg::new(" / ", Role::Meta), name];
    let mut right = Vec::new();
    if let Some(o) = overview {
        let s = &o.summary;
        segs.push(Seg::new("  ", Role::Body));
        let visibility = if s.private { "Private" } else { "Public" };
        segs.push(chip(visibility, Bg::SecondaryContainer));
        if s.archived {
            segs.push(space());
            segs.push(chip("Archived", Bg::TertiaryContainer));
        }
        let star = if o.starred { "★ Starred" } else { "☆ Star" };
        right.push(button(
            page,
            format!("{star}  {}", compact(s.stars)),
            Link::Star,
        ));
        for (text, n, tab) in [
            ("⑂ Fork", s.forks, "forks"),
            ("◉ Watch", o.watchers, "watchers"),
        ] {
            let target = format!("{}/{tab}", url::repo(repo));
            right.push(space());
            right.push(button(page, format!("{text} {}", compact(n)), target));
        }
    }
    page.add(Frame::None, 0, segs, right);
    if let Some(parent) = overview.and_then(|o| o.parent.as_deref()) {
        let link = RepoId::parse(parent)
            .map(|r| url::repo(&r))
            .unwrap_or_default();
        let parent = link_seg(page, parent.to_owned(), link, Role::Link);
        page.line(vec![Seg::new("forked from ", Role::Meta), parent]);
    }
    page.blank();
}

/// The About facts: description, homepage, topics, counts.
fn about(page: &mut Page, repo: &RepoId, o: &RepoOverview, compact_layout: bool) {
    if !compact_layout {
        page.line(vec![Seg::new("About", Role::Strong)]);
        page.blank();
    }
    match &o.summary.description {
        Some(d) => page.wrapped(vec![Seg::new(d.clone(), Role::Body)], 0, Frame::None),
        None if !compact_layout => page.line(vec![Seg::new(
            "No description, website, or topics provided.",
            Role::Meta,
        )]),
        None => {}
    }
    if let Some(home) = &o.homepage {
        let link = link_seg(page, home.clone(), home.clone(), Role::Link);
        page.line(vec![Seg::new("↗ ", Role::Meta), link]);
    }
    if !o.topics.is_empty() {
        let mut topics = Vec::new();
        for t in &o.topics {
            let topic = link_seg(
                page,
                format!(" {t} "),
                url::search(SearchKind::Repos, &format!("topic:{t}")),
                Role::Chip(Bg::PrimaryContainer),
            );
            topics.push(topic);
            topics.push(space());
        }
        page.blank();
        page.wrapped(topics, 0, Frame::None);
    }
    let mut facts = Vec::new();
    if let Some(license) = &o.license {
        facts.push(Seg::new(format!("⚖ {license} license"), Role::Meta));
    }
    for (icon, n, noun, tab) in [
        ("☆", o.summary.stars, "stars", "stargazers"),
        ("◉", o.watchers, "watching", "watchers"),
        ("⑂", o.summary.forks, "forks", "forks"),
    ] {
        let text = format!("{icon} {} {noun}", compact(n));
        let target = format!("{}/{tab}", url::repo(repo));
        facts.push(link_seg(page, text, target, Role::Meta));
    }
    if let Some(language) = &o.summary.language {
        facts.push(Seg::new(format!("● {language}"), Role::Meta));
    }
    page.blank();
    if compact_layout {
        let mut segs = Vec::new();
        for fact in facts {
            if !segs.is_empty() {
                segs.push(Seg::new("   ", Role::Meta));
            }
            segs.push(fact);
        }
        page.wrapped(segs, 0, Frame::None);
    } else {
        for fact in facts {
            page.line(vec![fact]);
        }
    }
}

/// `⎇ main ▾   ⌕ Go to file  t` above the file list or file.
fn code_toolbar(
    page: &mut Page,
    repo: &RepoId,
    rev: &str,
    path: &str,
    is_file: bool,
    commits: u64,
    keys: Keys<'_>,
) {
    let mut segs = vec![
        button(page, format!("⎇ {rev} ▾"), Link::Branch),
        Seg::new("  ", Role::Body),
    ];
    if is_file {
        segs.extend(crumbs(page, repo, rev, path, true));
    } else if !path.is_empty() {
        segs.extend(crumbs(page, repo, rev, path, false));
        segs.push(Seg::new("  ", Role::Body));
    }
    let mut right = go_to_file(page, keys);
    if commits > 0 {
        right.insert(0, space());
        right.insert(
            0,
            link_seg(
                page,
                format!("◷ {}", plural(commits, "commit")),
                url::commits(repo, rev, path),
                Role::Meta,
            ),
        );
    }
    page.add(Frame::None, 0, segs, right);
}

/// `repo / dir / sub /` with each part a link.
fn crumbs(page: &mut Page, repo: &RepoId, rev: &str, path: &str, last_is_file: bool) -> Vec<Seg> {
    let mut segs = vec![link_seg(
        page,
        repo.name.clone(),
        url::tree(repo, rev, ""),
        Role::Link,
    )];
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        segs.push(Seg::new(" / ", Role::Meta));
        if i + 1 == parts.len() {
            segs.push(Seg::new(*part, Role::Strong));
            if !last_is_file {
                segs.push(Seg::new(" /", Role::Meta));
            }
        } else {
            let sub = parts.get(..=i).unwrap_or_default().join("/");
            segs.push(link_seg(
                page,
                *part,
                url::tree(repo, rev, &sub),
                Role::Link,
            ));
        }
    }
    segs
}

/// The file list box.
fn file_box(page: &mut Page, dir: Listing<'_>, title: (Vec<Seg>, Vec<Seg>), cx: PageCtx<'_>) {
    let Listing {
        repo,
        rev,
        path,
        entries,
        commits,
    } = dir;
    let PageCtx { icons, now, .. } = cx;
    page.box_top(title.0, title.1);
    // GitHub's columns: name, the latest commit's message, its age.
    let name_w = entries
        .unwrap_or_default()
        .iter()
        .map(|e| crate::text::width(&e.name))
        .max()
        .unwrap_or(0)
        .min(usize::from(page.room(Frame::Body, 0)) / 3)
        + 4;
    if !path.is_empty() {
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        item(page, url::tree(repo, rev, parent), |page, link| {
            page.box_line(vec![Seg::linked("..", Role::Link, link)], Vec::new(), 2);
        });
    }
    match entries {
        Some([]) => empty_row(page, "This directory is empty."),
        None => skeleton(page),
        Some(entries) => {
            for e in entries {
                let (icon, icon_role, is_tree) = match e.kind {
                    EntryKind::Dir => (icons.folder(), Role::Accent, true),
                    EntryKind::Submodule => ("⊙", Role::Meta, true),
                    EntryKind::Symlink => ("↪", Role::Meta, false),
                    EntryKind::File => (icons.file(), Role::Meta, false),
                };
                let (target, role) = if is_tree {
                    (url::tree(repo, rev, &e.path), Role::Link)
                } else {
                    (url::blob(repo, rev, &e.path), Role::Body)
                };
                item(page, target, |page, link| {
                    let name = crate::text::truncate(&e.name, name_w - 4);
                    let pad = name_w.saturating_sub(crate::text::width(&name) + 2);
                    let mut segs = vec![
                        Seg::new(format!("{icon} "), icon_role),
                        Seg::linked(name, role, link),
                        Seg::new(" ".repeat(pad), Role::Body),
                    ];
                    let mut right = Vec::new();
                    if let Some(c) = commits.and_then(|m| m.get(&e.name)) {
                        let commit = page.link(url::commit(repo, &c.oid));
                        segs.push(Seg::linked(c.headline.clone(), Role::Meta, commit));
                        right.push(Seg::new(time::ago_iso(&c.date, now), Role::Meta));
                    }
                    page.box_line(segs, right, 0);
                });
            }
        }
    }
    page.box_bottom();
}

/// The Code tab at the repository root: files, README, About.
pub fn repo_code(
    page: &mut Page,
    repo: &RepoId,
    o: &RepoOverview,
    commits: Option<&HashMap<String, CommitInfo>>,
    aside: Option<u16>,
    cx: PageCtx<'_>,
) {
    let PageCtx { keys, now, .. } = cx;
    if aside.is_none() {
        about(page, repo, o, true);
        page.blank();
    }
    if o.entries.is_empty() && o.default_branch.is_none() {
        let title = Seg::new("This repository is empty.", Role::Strong);
        empty_box(page, title, "Nothing has been pushed yet.");
        return;
    }
    let rev = o.default_branch.clone().unwrap_or_else(|| "HEAD".into());
    code_toolbar(page, repo, &rev, "", false, o.commits, keys);
    let title = match &o.last_commit {
        Some(c) => {
            let author = link_seg(page, c.author.clone(), url::user(&c.author), Role::Strong);
            let headline = link_seg(
                page,
                c.headline.clone(),
                url::commit(repo, &c.oid),
                Role::Body,
            );
            (
                vec![author, Seg::new("  ", Role::Body), headline],
                vec![Seg::new(
                    format!(
                        "{} · {}",
                        crate::text::short_sha(&c.oid),
                        time::ago_iso(&c.date, now)
                    ),
                    Role::Meta,
                )],
            )
        }
        None => (vec![Seg::new("Files", Role::Strong)], Vec::new()),
    };
    let dir = Listing {
        repo,
        rev: &rev,
        path: "",
        entries: Some(&o.entries),
        commits,
    };
    file_box(page, dir, title, cx);
    if let Some(readme) = &o.readme {
        let name = link_seg(
            page,
            format!("☰ {}", readme.path),
            url::blob(repo, &rev, &readme.path),
            Role::Strong,
        );
        page.box_top(vec![name], Vec::new());
        let base = LinkBase::new(repo, rev, &readme.path);
        markdown::render(page, &readme.text, Some(&base), Frame::Body);
        page.box_bottom();
    }
    if let Some(width) = aside {
        page.build_aside(width, |a| about(a, repo, o, false));
    }
}

/// A directory below the root (or the root at another branch).
pub fn repo_dir(page: &mut Page, dir: Listing<'_>, cx: PageCtx<'_>) {
    let Listing {
        repo, rev, path, ..
    } = dir;
    code_toolbar(page, repo, rev, path, false, 0, cx.keys);
    let title = if path.is_empty() {
        format!("Files on {rev}")
    } else {
        path.rsplit('/').next().unwrap_or(path).to_owned()
    };
    file_box(
        page,
        dir,
        (vec![Seg::new(title, Role::Strong)], Vec::new()),
        cx,
    );
}

/// A file: highlighted with line numbers, or Markdown rendered.
/// A file at a revision, and the lines a link points at.
#[derive(Clone, Copy)]
pub struct FileAt<'a> {
    pub repo: &'a RepoId,
    pub rev: &'a str,
    pub path: &'a str,
    /// Marked, and where the page opens.
    pub lines: Option<(u32, u32)>,
}

pub fn file(page: &mut Page, at: FileAt<'_>, blob: &Blob, keys: Keys<'_>) {
    let FileAt {
        repo,
        rev,
        path,
        lines: marked,
    } = at;
    code_toolbar(page, repo, rev, path, true, 0, keys);
    let size = crate::text::size(blob.size);
    let Some(text) = &blob.text else {
        let title = Seg::new(size, Role::Meta);
        empty_box(page, title, "Binary file not shown. o opens it on GitHub.");
        return;
    };
    let lines = text.lines().count() as u64;
    let mut info = format!("{} · {size}", plural(lines, "line"));
    if blob.truncated {
        info.push_str(" · only the beginning is shown");
    }
    let blame = url::blob(repo, rev, path).replacen("/blob/", "/blame/", 1);
    let blame = link_seg(page, "Blame", blame, Role::Link);
    let raw = link_seg(
        page,
        "Raw",
        format!("https://raw.githubusercontent.com/{repo}/{rev}/{path}"),
        Role::Link,
    );
    let right = vec![blame, Seg::new("  ", Role::Meta), raw];
    page.box_top(vec![Seg::new(info, Role::Meta)], right);
    if path.to_ascii_lowercase().ends_with(".md") {
        let base = LinkBase::new(repo, rev, path);
        markdown::render(page, text, Some(&base), Frame::Body);
        page.box_bottom();
        return;
    }
    let lines = markdown::highlighted(text, ghtui_diff::Language::from_path(path));
    let width = lines.len().to_string().len();
    let is_marked = |n: usize| marked.is_some_and(|(a, b)| (a as usize..=b as usize).contains(&n));
    for (i, mut segs) in lines.into_iter().enumerate() {
        let number = format!("{:>width$}  ", i + 1);
        segs.insert(0, Seg::new(number, Role::Syntax(Syntax::Comment)));
        if is_marked(i + 1) && page.jump.is_none() {
            page.jump = Some(page.lines.len());
        }
        page.push(PageLine {
            segs,
            frame: Frame::Body,
            tone: if is_marked(i + 1) {
                Tone::Marked
            } else {
                Tone::Code
            },
            ..PageLine::default()
        });
    }
    page.box_bottom();
}

/// A file with, beside each run of lines, the commit that last changed
/// them: its ID, author and age.
pub fn blame(
    page: &mut Page,
    at: FileAt<'_>,
    blob: Option<&Blob>,
    blame: Option<&Blame>,
    keys: Keys<'_>,
    now: u64,
) {
    let FileAt {
        repo,
        rev,
        path,
        lines: marked,
    } = at;
    code_toolbar(page, repo, rev, path, true, 0, keys);
    let (Some(blob), Some(blame)) = (blob, blame) else {
        page.line(vec![Seg::new("Loading the blame…", Role::Meta)]);
        return;
    };
    let Some(text) = &blob.text else {
        empty_box(
            page,
            Seg::new("Blame", Role::Meta),
            "Binary file not shown.",
        );
        return;
    };
    let file = link_seg(page, "File", url::blob(repo, rev, path), Role::Link);
    let info = format!("Blame · {}", plural(blame.ranges.len() as u64, "change"));
    page.box_top(vec![Seg::new(info, Role::Meta)], vec![file]);
    let lines = markdown::highlighted(text, ghtui_diff::Language::from_path(path));
    let width = lines.len().to_string().len();
    let is_marked = |n: usize| marked.is_some_and(|(a, b)| (a as usize..=b as usize).contains(&n));
    for (i, mut segs) in lines.into_iter().enumerate() {
        let n = i + 1;
        let range = blame
            .ranges
            .iter()
            .find(|r| (r.start as usize..=r.end as usize).contains(&n));
        let mut gutter = match range {
            Some(r) if r.start as usize == n => {
                let author: String = r.author.chars().take(12).collect();
                vec![
                    link_seg(
                        page,
                        crate::text::short_sha(&r.oid),
                        url::commit(repo, &r.oid),
                        Role::Code,
                    ),
                    Seg::new(
                        format!(" {author:<12} {:>8}  ", time::ago_iso(&r.date, now)),
                        Role::Meta,
                    ),
                ]
            }
            _ => vec![Seg::new(" ".repeat(31), Role::Meta)],
        };
        gutter.push(Seg::new(
            format!("{n:>width$}  "),
            Role::Syntax(Syntax::Comment),
        ));
        gutter.append(&mut segs);
        if is_marked(n) && page.jump.is_none() {
            page.jump = Some(page.lines.len());
        }
        if range.is_some_and(|r| r.start as usize == n && n > 1) {
            page.box_rule();
        }
        page.push(PageLine {
            segs: gutter,
            frame: Frame::Body,
            tone: if is_marked(n) {
                Tone::Marked
            } else {
                Tone::Code
            },
            ..PageLine::default()
        });
    }
    page.box_bottom();
}
// ---- security advisories ----------------------------------------------------------------------

fn severity_chip(severity: &str) -> Seg {
    let bg = match severity {
        "critical" | "high" => Bg::ErrorContainer,
        "medium" => Bg::TertiaryContainer,
        _ => Bg::SecondaryContainer,
    };
    let mut name = severity.to_owned();
    if let Some(first) = name.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    chip(name, bg)
}

/// A repository's security advisories, or GitHub's newest reviewed ones.
pub fn advisories(page: &mut Page, repo: Option<&RepoId>, list: Option<&Vec<Advisory>>, now: u64) {
    let Some(l) = list else {
        page.line(vec![Seg::new("Loading advisories…", Role::Meta)]);
        return;
    };
    let title = vec![Seg::new(
        format!("Security advisories  {}", l.len()),
        Role::Strong,
    )];
    let empty = if repo.is_some() {
        "No published advisories."
    } else {
        "No advisories."
    };
    list_box(page, title, Vec::new(), l, empty, |page, a| {
        let target = match repo {
            Some(repo) => format!("{}/security/advisories/{}", url::repo(repo), a.ghsa),
            None => format!("{}/advisories/{}", url::BASE, a.ghsa),
        };
        item(page, target, |page, link| {
            let segs = vec![
                severity_chip(&a.severity),
                space(),
                Seg::linked(a.summary.clone(), Role::Strong, link),
            ];
            hanging(page, Seg::new("", Role::Meta), segs, Frame::Body);
            let mut meta = a.ghsa.clone();
            if let Some(at) = &a.published_at {
                meta.push_str(&format!(" · published {}", time::ago_iso(at, now)));
            }
            let packages = unique(a.packages.iter().map(|p| &p.name));
            if !packages.is_empty() {
                meta.push_str(&format!(" · {}", packages.join(", ")));
            }
            body(page, vec![Seg::new(meta, Role::Meta)]);
        });
    });
}

/// A security advisory: what's affected and fixed, how severe, and its
/// description, credits and references.
pub fn advisory(page: &mut Page, a: &Advisory, now: u64) {
    page.wrapped(
        vec![Seg::new(a.summary.clone(), Role::Title)],
        0,
        Frame::None,
    );
    let mut segs = vec![severity_chip(&a.severity)];
    if a.withdrawn_at.is_some() {
        segs.push(space());
        segs.push(chip("Withdrawn", Bg::SecondaryContainer));
    }
    let mut meta = format!("  {}", a.ghsa);
    if let Some(cve) = &a.cve {
        meta.push_str(&format!(" · {cve}"));
    }
    if let Some(at) = &a.published_at {
        meta.push_str(&format!(" · published {}", time::ago_iso(at, now)));
    }
    if let Some(at) = &a.updated_at {
        meta.push_str(&format!(" · updated {}", time::ago_iso(at, now)));
    }
    segs.push(Seg::new(meta, Role::Meta));
    page.wrapped(segs, 0, Frame::None);
    if let Some((score, vector)) = &a.cvss {
        page.wrapped(
            vec![Seg::new(format!("CVSS {score}  {vector}"), Role::Meta)],
            0,
            Frame::None,
        );
    }
    if !a.cwes.is_empty() {
        page.wrapped(
            vec![Seg::new(a.cwes.join(" · "), Role::Meta)],
            0,
            Frame::None,
        );
    }
    page.blank();
    let title = vec![Seg::new(
        format!("Affected packages  {}", a.packages.len()),
        Role::Strong,
    )];
    list_box(
        page,
        title,
        Vec::new(),
        &a.packages,
        "None listed.",
        |page, p| {
            let right = vec![Seg::new(p.ecosystem.clone(), Role::Meta)];
            page.box_line(vec![Seg::new(p.name.clone(), Role::Strong)], right, 0);
            let mut facts = Vec::new();
            if let Some(v) = &p.vulnerable {
                facts.push(format!("Affected {v}"));
            }
            facts.push(format!(
                "Patched {}",
                p.patched.as_deref().unwrap_or("in no version yet")
            ));
            body(page, vec![Seg::new(facts.join(" · "), Role::Meta)]);
        },
    );
    if !a.description.trim().is_empty() {
        page.blank();
        markdown::render(page, &a.description, None, Frame::None);
    }
    if !a.credits.is_empty() {
        page.blank();
        let mut segs = vec![Seg::new("Credits  ", Role::Strong)];
        for (i, login) in a.credits.iter().enumerate() {
            if i > 0 {
                segs.push(Seg::new(", ", Role::Meta));
            }
            segs.push(link_seg(page, login.clone(), url::user(login), Role::Link));
        }
        page.wrapped(segs, 0, Frame::None);
    }
    if !a.references.is_empty() {
        page.blank();
        aside_heading(page, "References");
        for r in &a.references {
            let link = link_seg(page, r.clone(), r.clone(), Role::Link);
            page.wrapped(vec![link], 0, Frame::None);
        }
    }
}
// ---- wikis -------------------------------------------------------------------------------------

/// A wiki's `[[Page]]` and `[[Text|Page]]` links as Markdown links.
fn wiki_links(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("[[")
        && let Some((inner, tail)) = after.split_once("]]")
    {
        let (label, target) = inner.split_once('|').unwrap_or((inner, inner));
        out.push_str(before);
        out.push_str(&format!("[{label}]({})", target.trim().replace(' ', "-")));
        rest = tail;
    }
    out.push_str(rest);
    out
}
/// A wiki page, then the wiki's pages and its sidebar. Without a page,
/// just the pages.
pub fn wiki(page: &mut Page, repo: &RepoId, w: &WikiPage) {
    let base = LinkBase::wiki(repo);
    if let (Some(title), Some(text)) = (&w.title, &w.text) {
        page.wrapped(vec![Seg::new(title.clone(), Role::Title)], 0, Frame::None);
        page.rule(0, Frame::None);
        if w.markdown {
            markdown::render(page, &wiki_links(text), Some(&base), Frame::None);
        } else {
            for line in text.lines() {
                page.wrapped(vec![Seg::new(line.to_owned(), Role::Body)], 0, Frame::None);
            }
        }
        page.blank();
    }
    let title = vec![Seg::new(format!("Pages  {}", w.pages.len()), Role::Strong)];
    list_box(
        page,
        title,
        Vec::new(),
        &w.pages,
        "No pages yet.",
        |page, title| {
            let target = format!(
                "{}/wiki/{}",
                url::repo(repo),
                url::encode_path(&title.replace(' ', "-"))
            );
            item(page, target, |page, link| {
                page.box_line(
                    vec![Seg::linked(title.clone(), Role::Link, link)],
                    Vec::new(),
                    0,
                );
            });
        },
    );
    if let Some(sidebar) = &w.sidebar {
        page.blank();
        markdown::render(page, &wiki_links(sidebar), Some(&base), Frame::None);
    }
}
// ---- teams -------------------------------------------------------------------------------------

fn team_row(page: &mut Page, org: &str, t: &TeamSummary) {
    item(
        page,
        format!("{}/orgs/{org}/teams/{}", url::BASE, t.slug),
        |page, link| {
            let mut segs = vec![Seg::linked(t.name.clone(), Role::Strong, link)];
            if t.secret {
                segs.push(space());
                segs.push(chip("Secret", Bg::SecondaryContainer));
            }
            let counts = format!(
                "{} · {}",
                plural(t.members, "member"),
                if t.repos == 1 {
                    "1 repository".to_owned()
                } else {
                    format!("{} repositories", t.repos)
                }
            );
            page.box_line(segs, vec![Seg::new(counts, Role::Meta)], 0);
            if !t.description.is_empty() {
                body(page, vec![Seg::new(t.description.clone(), Role::Meta)]);
            }
        },
    );
}

/// An organization's teams, the ones you can see.
pub fn teams(page: &mut Page, org: &str, list: Option<&Results<TeamSummary>>) {
    let Some(l) = list else {
        page.line(vec![Seg::new("Loading teams…", Role::Meta)]);
        return;
    };
    let title = vec![Seg::new(
        format!("Teams  {}", compact(l.total)),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    if l.items.is_empty() {
        empty_row(
            page,
            "No teams you can see: an organization's teams show to its members.",
        );
    }
    box_rows(page, &l.items, |page, t| team_row(page, org, t));
    more_row(page, l.next.is_some(), l.items.len(), l.total);
    page.box_bottom();
}

/// A team: its description and parent, then its members, repositories
/// and child teams.
pub fn team(page: &mut Page, org: &str, d: &TeamDetail) {
    let t = &d.team;
    let mut title = vec![
        link_seg(page, format!("@{org}"), url::user(org), Role::Link),
        Seg::new(" / ", Role::Meta),
        Seg::new(t.name.clone(), Role::Title),
    ];
    if t.secret {
        title.push(space());
        title.push(chip("Secret", Bg::SecondaryContainer));
    }
    page.wrapped(title, 0, Frame::None);
    if !t.description.is_empty() {
        page.wrapped(
            vec![Seg::new(t.description.clone(), Role::Body)],
            0,
            Frame::None,
        );
    }
    if let Some(parent) = &d.parent {
        let target = format!("{}/orgs/{org}/teams/{}", url::BASE, parent.slug);
        let link = link_seg(page, parent.name.clone(), target, Role::Link);
        page.wrapped(vec![Seg::new("Part of ", Role::Meta), link], 0, Frame::None);
    }
    page.blank();
    let shown = |n: usize, total: u64| {
        if (n as u64) < total {
            format!("{n} of {total}")
        } else {
            total.to_string()
        }
    };
    let title = vec![Seg::new(
        format!("Members  {}", shown(d.members.len(), t.members)),
        Role::Strong,
    )];
    list_box(page, title, Vec::new(), &d.members, "No members.", user_row);
    page.blank();
    let title = vec![Seg::new(
        format!("Repositories  {}", shown(d.repos.len(), t.repos)),
        Role::Strong,
    )];
    list_box(
        page,
        title,
        Vec::new(),
        &d.repos,
        "No repositories.",
        |page, r| {
            item(page, url::repo(&r.repo), |page, link| {
                let right = vec![Seg::new(format!("★ {}", compact(r.stars)), Role::Meta)];
                page.box_line(
                    vec![Seg::linked(r.repo.to_string(), Role::Link, link)],
                    right,
                    0,
                );
                if !r.description.is_empty() {
                    body(page, vec![Seg::new(r.description.clone(), Role::Meta)]);
                }
            });
        },
    );
    if !d.children.is_empty() {
        page.blank();
        let title = vec![Seg::new(
            format!("Child teams  {}", d.children.len()),
            Role::Strong,
        )];
        list_box(page, title, Vec::new(), &d.children, "", |page, c| {
            team_row(page, org, c);
        });
    }
}
// ---- gists -------------------------------------------------------------------------------------

const GIST: &str = "https://gist.github.com";

/// A gist: who and when, its description, and each of its files.
pub fn gist(page: &mut Page, g: &Gist, now: u64) {
    let first = g.files.first().map_or(g.id.as_str(), |f| f.name.as_str());
    let mut title = Vec::new();
    if let Some(owner) = &g.owner {
        title.push(link_seg(
            page,
            owner.clone(),
            format!("{GIST}/{owner}"),
            Role::Link,
        ));
        title.push(Seg::new(" / ", Role::Meta));
    }
    title.push(Seg::new(first.to_owned(), Role::Title));
    if !g.public {
        title.push(space());
        title.push(chip("Secret", Bg::SecondaryContainer));
    }
    page.wrapped(title, 0, Frame::None);
    let meta = format!(
        "created {} · updated {} · {} · {}",
        time::ago_iso(&g.created_at, now),
        time::ago_iso(&g.updated_at, now),
        plural(g.files.len() as u64, "file"),
        plural(g.comments, "comment"),
    );
    page.wrapped(vec![Seg::new(meta, Role::Meta)], 0, Frame::None);
    if !g.description.is_empty() {
        page.wrapped(
            vec![Seg::new(g.description.clone(), Role::Body)],
            0,
            Frame::None,
        );
    }
    for f in &g.files {
        page.blank();
        let mut info = vec![Seg::new(f.name.clone(), Role::Strong)];
        let mut facts = vec![crate::text::size(f.size)];
        if let Some(language) = &f.language {
            facts.push(language.clone());
        }
        info.push(Seg::new(format!("  {}", facts.join(" · ")), Role::Meta));
        page.box_top(info, Vec::new());
        match &f.text {
            Some(text) if f.name.to_ascii_lowercase().ends_with(".md") => {
                markdown::render(page, text, None, Frame::Body);
            }
            Some(text) => {
                let lines = markdown::highlighted(text, ghtui_diff::Language::from_path(&f.name));
                let width = lines.len().to_string().len();
                for (i, mut segs) in lines.into_iter().enumerate() {
                    segs.insert(
                        0,
                        Seg::new(
                            format!("{:>width$}  ", i + 1),
                            Role::Syntax(Syntax::Comment),
                        ),
                    );
                    page.push(PageLine {
                        segs,
                        frame: Frame::Body,
                        tone: Tone::Code,
                        ..PageLine::default()
                    });
                }
            }
            None => empty_row(page, "Too large to show here. o opens it on GitHub."),
        }
        if f.truncated {
            body(
                page,
                vec![Seg::new("Only the beginning is shown.", Role::Meta)],
            );
        }
        page.box_bottom();
    }
}

/// Someone's public gists, most recently updated first.
pub fn gists(page: &mut Page, login: &str, list: Option<&Results<GistSummary>>, now: u64) {
    let Some(l) = list else {
        page.line(vec![Seg::new("Loading gists…", Role::Meta)]);
        return;
    };
    let title = vec![Seg::new(
        format!("Gists  {}", compact(l.total)),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    if l.items.is_empty() {
        empty_row(page, "No public gists.");
    }
    box_rows(page, &l.items, |page, g| {
        item(page, format!("{GIST}/{login}/{}", g.id), |page, link| {
            let name = g.files.first().cloned().unwrap_or_else(|| g.id.clone());
            let right = vec![Seg::new(time::ago_iso(&g.updated_at, now), Role::Meta)];
            page.box_line(vec![Seg::linked(name, Role::Strong, link)], right, 0);
            if !g.description.is_empty() {
                body(page, vec![Seg::new(g.description.clone(), Role::Body)]);
            }
            let meta = format!(
                "{} · {} · ★ {}",
                plural(g.files.len() as u64, "file"),
                plural(g.comments, "comment"),
                compact(g.stars)
            );
            body(page, vec![Seg::new(meta, Role::Meta)]);
        });
    });
    more_row(page, l.next.is_some(), l.items.len(), l.total);
    page.box_bottom();
}
// ---- lists --------------------------------------------------------------------------

fn repo_row(page: &mut Page, r: &RepoSummary, now: u64, show_owner: bool) {
    let name = if show_owner {
        r.repo.to_string()
    } else {
        r.repo.name.clone()
    };
    item(page, url::repo(&r.repo), |page, link| {
        let mut segs = vec![Seg::linked(name, Role::Link, link)];
        for (on, text) in [
            (r.private, "Private"),
            (r.fork, "Fork"),
            (r.archived, "Archived"),
        ] {
            if on {
                segs.push(space());
                segs.push(chip(text, Bg::SecondaryContainer));
            }
        }
        let right = vec![Seg::new(format!("☆ {}", compact(r.stars)), Role::Meta)];
        page.box_line(segs, right, 0);
        if page.compact {
            return;
        }
        if let Some(d) = &r.description {
            page.wrapped(vec![Seg::new(d.clone(), Role::Body)], 0, Frame::Body);
        }
        let mut meta = Vec::new();
        if let Some(lang) = &r.language {
            meta.push(Seg::new("● ", Role::Accent));
            meta.push(Seg::new(format!("{lang}   "), Role::Meta));
        }
        meta.push(Seg::new(format!("⑂ {}", compact(r.forks)), Role::Meta));
        if let Some(p) = &r.pushed_at {
            let updated = format!("   Updated {}", time::ago_iso(p, now));
            meta.push(Seg::new(updated, Role::Meta));
        }
        body(page, meta);
    });
}

fn issue_row(page: &mut Page, i: &IssueSummary, show_repo: bool, icons: Icons, now: u64) {
    let (icon, role) = issue_icon(icons, i.state, i.is_pr);
    let target = if i.is_pr {
        url::pull(&PrRef {
            repo: i.repo.clone(),
            number: i.number,
        })
    } else {
        url::issue(&i.repo, i.number)
    };
    let place = if show_repo {
        format!("{}#{}", i.repo, i.number)
    } else {
        format!("#{}", i.number)
    };
    item(page, target, |page, link| {
        if page.compact {
            // One row: the title, then where it is and its signals.
            let segs = vec![
                Seg::new(format!("{icon} "), role),
                Seg::linked(i.title.clone(), Role::Strong, link),
            ];
            let mut right = vec![Seg::new(format!("{place}  "), Role::Meta)];
            right.extend(issue_signals(i, icons));
            page.box_line(segs, right, 0);
            return;
        }
        let mut segs = vec![Seg::linked(i.title.clone(), Role::Strong, link)];
        labels(&mut segs, &i.labels);
        hanging(page, Seg::new(format!("{icon} "), role), segs, Frame::Body);
        // GitHub's second line: `#12 opened 3 days ago by octocat · Approved`.
        let (verb, at) = if i.created_at.is_empty() {
            ("updated", &i.updated_at)
        } else {
            ("opened", &i.created_at)
        };
        let when = format!("{place} {verb} {} by {}", time::ago_iso(at, now), i.author);
        let mut meta = vec![Seg::new(when, Role::Meta)];
        if let Some(review) = i.review {
            let (text, role) = match review {
                ReviewDecision::Approved => ("Approved", Role::Success),
                ReviewDecision::ChangesRequested => ("Changes requested", Role::Error),
                ReviewDecision::ReviewRequired => ("Review required", Role::Meta),
            };
            meta.extend([Seg::new(" · ", Role::Meta), Seg::new(text, role)]);
        }
        page.box_line(meta, issue_signals(i, icons), 2);
    });
}

/// Checks and comment count, for the right of an issue's row.
fn issue_signals(i: &IssueSummary, icons: Icons) -> Vec<Seg> {
    let mut right = Vec::new();
    if let Some(checks) = i.checks {
        let role = match checks {
            ChecksState::Passing => Role::Success,
            ChecksState::Failing => Role::Error,
            ChecksState::Pending => Role::Accent,
        };
        right.push(Seg::new(format!("{}  ", icons.checks(checks)), role));
    }
    if i.comments > 0 {
        right.push(Seg::new(
            format!("{} {}", icons.comment(), i.comments),
            Role::Meta,
        ));
    }
    right
}

fn user_row(page: &mut Page, u: &UserSummary) {
    item(page, url::user(&u.login), |page, link| {
        let mut segs = vec![Seg::linked(u.login.clone(), Role::Link, link)];
        if let Some(name) = &u.name {
            segs.push(Seg::new(format!("  {name}"), Role::Strong));
        }
        if u.is_org {
            segs.push(space());
            segs.push(chip("Organization", Bg::SecondaryContainer));
        }
        body(page, segs);
        if let Some(bio) = u.bio.as_ref().filter(|_| !page.compact) {
            page.wrapped(vec![Seg::new(bio.clone(), Role::Meta)], 0, Frame::Body);
        }
    });
}

/// A list of people (stargazers, watchers, followers…) that loads more.
pub fn people_list(page: &mut Page, title: &str, people: Option<&Results<UserSummary>>) {
    let Some(r) = people else {
        page.line(vec![Seg::new("Loading…", Role::Meta)]);
        return;
    };
    let title = format!("{title}  {}", compact(r.total));
    page.box_top(vec![Seg::new(title, Role::Strong)], Vec::new());
    if r.items.is_empty() {
        empty_row(page, "Nobody here yet.");
    }
    box_rows(page, &r.items, user_row);
    more_row(page, r.next.is_some(), r.items.len(), r.total);
    page.box_bottom();
}

/// A list of repositories (forks…) that loads more.
pub fn repos(page: &mut Page, title: &str, repos: Option<&Results<RepoSummary>>, now: u64) {
    let total = repos.map_or(String::new(), |r| format!("  {}", compact(r.total)));
    let title = vec![Seg::new(format!("{title}{total}"), Role::Strong)];
    repo_list_box(page, title, Vec::new(), repos, true, "None yet.", now);
}

/// Repositories in a box that loads more.
fn repo_list_box(
    page: &mut Page,
    title: Vec<Seg>,
    right: Vec<Seg>,
    repos: Option<&Results<RepoSummary>>,
    show_owner: bool,
    empty: &str,
    now: u64,
) {
    let Some(r) = repos else {
        page.line(vec![Seg::new("Loading…", Role::Meta)]);
        return;
    };
    page.box_top(title, right);
    if r.items.is_empty() {
        empty_row(page, empty);
    }
    box_rows(page, &r.items, |page, repo| {
        repo_row(page, repo, now, show_owner);
    });
    more_row(page, r.next.is_some(), r.items.len(), r.total);
    page.box_bottom();
}

fn more_row(page: &mut Page, next: bool, shown: usize, total: u64) {
    if !next {
        return;
    }
    page.box_rule();
    item(page, Link::More, |page, link| {
        let text = format!("Load more  ({shown} of {})", compact(total));
        body(page, vec![Seg::linked(text, Role::Link, link)]);
    });
}

/// The filter field above a list: `⌕ is:open label:bug`.
fn filter_field(page: &mut Page, query: &str, keys: Keys<'_>) {
    let width = usize::from(page.room(Frame::None, 0));
    let hint = format!("{} to filter ", keys.filter);
    let text = format!(" ⌕ {query}");
    let pad = width.saturating_sub(crate::text::width(&text) + crate::text::width(&hint));
    let link = page.link(Link::Filter);
    page.line(vec![
        Seg::linked(
            format!("{text}{}", " ".repeat(pad)),
            Role::Chip(Bg::ContainerHigh),
            link,
        ),
        Seg::linked(hint, Role::Chip(Bg::ContainerHigh), link),
    ]);
}

/// Which list state a query shows.
pub fn list_state(query: &str) -> &'static str {
    let has = |state: &str| {
        query
            .split_whitespace()
            .any(|w| w.strip_prefix("is:").or_else(|| w.strip_prefix("state:")) == Some(state))
    };
    if has("closed") {
        "closed"
    } else if has("open") {
        "open"
    } else {
        "all"
    }
}

/// The sort a query asks for, by name.
pub fn sort_label(query: &str) -> &'static str {
    let sort = query
        .split_whitespace()
        .find_map(|w| w.strip_prefix("sort:"))
        .unwrap_or("created-desc");
    SORTS
        .iter()
        .find(|(q, _)| *q == sort)
        .map_or("Best match", |(_, l)| l)
}

/// GitHub's sorts: `(qualifier, label)`.
pub const SORTS: [(&str, &str); 5] = [
    ("created-desc", "Newest"),
    ("created-asc", "Oldest"),
    ("comments-desc", "Most commented"),
    ("updated-desc", "Recently updated"),
    ("reactions-desc", "Most reactions"),
];

/// A repository's issue or pull request list.
pub fn issue_list(
    page: &mut Page,
    query: &str,
    counts: Option<(u64, u64)>,
    is_pr: bool,
    results: Option<&Results<IssueSummary>>,
    cx: PageCtx<'_>,
) {
    let PageCtx { icons, keys, now } = cx;
    filter_field(page, query, keys);
    let state = list_state(query);
    let mut title = Vec::new();
    let closed_kind = if is_pr {
        IssueState::Merged
    } else {
        IssueState::Closed
    };
    let (open, closed) = counts.unwrap_or_default();
    for (name, kind, n, label) in [
        ("open", IssueState::Open, open, "Open"),
        ("closed", closed_kind, closed, "Closed"),
    ] {
        let (icon, _) = issue_icon(icons, kind, is_pr);
        let role = if state == name {
            Role::Strong
        } else {
            Role::Meta
        };
        let text = match counts {
            Some(_) => format!("{icon} {} {label}", compact(n)),
            None => format!("{icon} {label}"),
        };
        let link = page.link(Link::State(name.to_owned()));
        if !title.is_empty() {
            title.push(Seg::new("   ", Role::Body));
        }
        title.push(Seg::linked(text, role, link));
    }
    let sort = link_seg(
        page,
        format!("Sort: {} ▾", sort_label(query)),
        Link::Sort,
        Role::Meta,
    );
    page.box_top(title, vec![sort]);
    match results {
        None => skeleton(page),
        Some(r) if r.items.is_empty() => {
            empty_row(page, "No results matched your search.");
        }
        Some(r) => {
            box_rows(page, &r.items, |page, i| {
                issue_row(page, i, false, icons, now);
            });
            more_row(page, r.next.is_some(), r.items.len(), r.total);
        }
    }
    page.box_bottom();
}

// ---- search -------------------------------------------------------------------------

pub fn search(
    page: &mut Page,
    kind: SearchKind,
    query: &str,
    results: Option<&SearchResults>,
    icons: Icons,
    keys: Keys<'_>,
    now: u64,
) {
    filter_field(page, query, keys);
    let noun = match kind {
        SearchKind::Repos => "repository",
        SearchKind::Issues => "issue",
        SearchKind::Pulls => "pull request",
        SearchKind::Users => "user",
        SearchKind::Discussions => "discussion",
        SearchKind::Commits => "commit",
        SearchKind::Code => "code",
    };
    let Some(results) = results else {
        loading_box(page, vec![Seg::new("Searching…", Role::Meta)]);
        return;
    };
    let (total, shown) = results.counts();
    let next = results.next().is_some();
    page.box_top(
        vec![Seg::new(
            format!("{} {noun} results", compact(total)),
            Role::Strong,
        )],
        Vec::new(),
    );
    if shown == 0 {
        empty_row(page, "Your search did not match anything.");
    }
    match results {
        SearchResults::Repos(r) => box_rows(page, &r.items, |page, r| repo_row(page, r, now, true)),
        SearchResults::Issues(r) => box_rows(page, &r.items, |page, i| {
            issue_row(page, i, true, icons, now);
        }),
        SearchResults::Users(r) => box_rows(page, &r.items, user_row),
        SearchResults::Discussions(r) => box_rows(page, &r.items, |page, hit| {
            let d = &hit.summary;
            let target = format!("{}/discussions/{}", url::repo(&hit.repo), d.number);
            item(page, target, |page, link| {
                let mut segs = vec![Seg::linked(d.title.clone(), Role::Strong, link)];
                if d.answered {
                    segs.push(space());
                    segs.push(chip("✓ Answered", Bg::SuccessContainer));
                }
                body(page, segs);
                let meta = format!(
                    "{}#{} · {} · {} · {} · updated {}",
                    hit.repo,
                    d.number,
                    d.category,
                    d.author,
                    plural(d.comments, "comment"),
                    time::ago_iso(&d.updated_at, now)
                );
                body(page, vec![Seg::new(meta, Role::Meta)]);
            });
        }),
        SearchResults::Commits(r) => box_rows(page, &r.items, |page, hit| {
            let c = &hit.commit;
            item(page, url::commit(&hit.repo, &c.oid), |page, link| {
                let right = vec![Seg::new(crate::text::short_sha(&c.oid), Role::Code)];
                page.box_line(
                    vec![Seg::linked(c.headline.clone(), Role::Strong, link)],
                    right,
                    0,
                );
                let meta = format!(
                    "{} · {} committed {}",
                    hit.repo,
                    c.author,
                    time::ago_iso(&c.date, now)
                );
                body(page, vec![Seg::new(meta, Role::Meta)]);
            });
        }),
        SearchResults::Code(r) => box_rows(page, &r.items, |page, hit| {
            item(
                page,
                url::blob(&hit.repo, "HEAD", &hit.path),
                |page, link| {
                    page.box_line(
                        vec![Seg::linked(hit.path.clone(), Role::Code, link)],
                        Vec::new(),
                        0,
                    );
                    body(page, vec![Seg::new(hit.repo.to_string(), Role::Meta)]);
                },
            );
        }),
    }
    more_row(page, next, shown, total);
    page.box_bottom();
}

// ---- conversations --------------------------------------------------------------------

/// What the comment boxes of a conversation share.
struct Conversation<'a> {
    base: LinkBase,
    /// Who opened it.
    op: &'a str,
    now: u64,
}

impl Conversation<'_> {
    /// A comment box: `╭─ author commented 3d ago ─── Author ─╮`, the
    /// Markdown body, `╰──╯`. `verb` is "commented", "opened"...
    fn said(&self, page: &mut Page, author: &str, verb: &str, when: &str, body: &str, badge: bool) {
        self.said_at(
            page,
            None,
            author,
            verb,
            when,
            body,
            badge.then(|| chip("Author", Bg::SecondaryContainer)),
        );
    }

    /// A comment box, named `anchor` for links to it, with `chip` on its
    /// right (Author, Answer).
    #[expect(clippy::too_many_arguments, reason = "the parts of a comment box")]
    fn said_at(
        &self,
        page: &mut Page,
        anchor: Option<String>,
        author: &str,
        verb: &str,
        when: &str,
        body: &str,
        chip: Option<Seg>,
    ) {
        if let Some(anchor) = anchor {
            page.anchor(anchor);
        }
        let start = page.lines.len();
        let who = link_seg(page, author.to_owned(), url::user(author), Role::Strong);
        let when = time::ago_iso(when, self.now);
        let right = Vec::from_iter(chip);
        page.box_top(
            vec![who, Seg::new(format!(" {verb} {when}"), Role::Meta)],
            right,
        );
        if body.trim().is_empty() {
            empty_row(page, "No description provided.");
        } else {
            markdown::render(page, body, Some(&self.base), Frame::Body);
        }
        page.box_bottom();
        // The whole comment is a row: Enter quote-replies, as GitHub's `r` does.
        let quote = page.quote(author, body);
        page.item(start, quote);
    }

    /// A comment, badged when it's by whoever opened the conversation.
    fn comment(&self, page: &mut Page, c: &Comment) {
        let anchor = c.id.map(|id| format!("issuecomment-{id}"));
        let badge = (c.author == self.op).then(|| chip("Author", Bg::SecondaryContainer));
        self.said_at(
            page,
            anchor,
            &c.author,
            "commented",
            &c.created_at,
            &c.body,
            badge,
        );
    }

    /// A timeline event: `● author approved these changes 1d ago`.
    fn event(
        &self,
        page: &mut Page,
        (icon, role): (&str, Role),
        author: &str,
        what: &str,
        when: &str,
    ) {
        let who = link_seg(page, author.to_owned(), url::user(author), Role::Strong);
        let when = time::ago_iso(when, self.now);
        let what = Seg::new(format!(" {what} {when}"), Role::Meta);
        page.add(
            Frame::None,
            2,
            vec![Seg::new(format!("{icon}  "), role), who, what],
            Vec::new(),
        );
    }
}

/// The line between timeline entries.
fn connector(page: &mut Page) {
    page.add(Frame::None, 3, vec![Seg::new("│", Role::Meta)], Vec::new());
}

/// The comment box at the end of a conversation.
fn add_comment(page: &mut Page, keys: Keys<'_>) {
    connector(page);
    item(page, Link::Comment, |page, link| {
        page.box_top(
            vec![Seg::linked("Add a comment", Role::Strong, link)],
            Vec::new(),
        );
        let press = format!(
            "Press {} to write a comment (Markdown; ctrl-e for $EDITOR)",
            keys.comment
        );
        body(page, vec![Seg::linked(press, Role::Meta, link)]);
        page.box_bottom();
    });
}

fn people(page: &mut Page, heading: &str, logins: &[String], none: &str) {
    aside_heading(page, heading);
    if logins.is_empty() {
        page.line(vec![Seg::new(none, Role::Meta)]);
    }
    for login in logins {
        page.link_line(login.clone(), url::user(login), Role::Link, 0);
    }
}

fn label_list(page: &mut Page, l: &[Label]) {
    aside_heading(page, "Labels");
    if l.is_empty() {
        page.line(vec![Seg::new("None yet", Role::Meta)]);
    } else {
        let mut segs = Vec::new();
        labels(&mut segs, l);
        page.wrapped(segs, 0, Frame::None);
    }
}

pub fn issue(
    page: &mut Page,
    d: &IssueDetail,
    icons: Icons,
    keys: Keys<'_>,
    aside: Option<u16>,
    now: u64,
) {
    page.wrapped(
        vec![
            Seg::new(d.title.clone(), Role::Title),
            Seg::new(format!("  #{}", d.number), Role::Meta),
        ],
        0,
        Frame::None,
    );
    let author = link_seg(page, d.author.clone(), url::user(&d.author), Role::Strong);
    let mut segs = vec![
        state_chip(d.state, false, icons),
        Seg::new("  ", Role::Body),
        author,
        Seg::new(
            format!(
                " opened this issue {} · {}",
                time::ago_iso(&d.created_at, now),
                plural(d.comments.len() as u64, "comment")
            ),
            Role::Meta,
        ),
    ];
    if aside.is_none() {
        labels(&mut segs, &d.labels);
    }
    page.wrapped(segs, 0, Frame::None);
    if aside.is_some() || d.assignees.is_empty() {
        page.rule(0, Frame::None);
    }
    if aside.is_none() && !d.assignees.is_empty() {
        let mut segs = vec![Seg::new("Assignees: ", Role::Meta)];
        for (i, a) in d.assignees.iter().enumerate() {
            if i > 0 {
                segs.push(Seg::new(", ", Role::Meta));
            }
            segs.push(link_seg(page, a.clone(), url::user(a), Role::Link));
        }
        page.line(segs);
    }
    let talk = Conversation {
        base: LinkBase::new(&d.repo, "HEAD", ""),
        op: &d.author,
        now,
    };
    talk.said(page, &d.author, "opened", &d.created_at, &d.body, true);
    for c in &d.comments {
        connector(page);
        talk.comment(page, c);
    }
    add_comment(page, keys);
    if let Some(width) = aside {
        let participants =
            unique(std::iter::once(&d.author).chain(d.comments.iter().map(|c| &c.author)));
        page.build_aside(width, |a| {
            people(a, "Assignees", &d.assignees, "No one assigned");
            label_list(a, &d.labels);
            people(a, "Participants", &participants, "");
        });
    }
}

// ---- pull request -------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrTab {
    Conversation,
    Commits,
    Checks,
}

/// The lines under a PR's title: branches, size (the state is in the
/// sticky title above).
fn pr_summary(page: &mut Page, pr: &PrRef, d: &PrDetail, now: u64) {
    let s = &d.summary;
    let author = link_seg(page, s.author.clone(), url::user(&s.author), Role::Strong);
    let head = match &d.head_repo {
        Some(repo) if *repo != pr.repo.to_string() => {
            format!("{}:{}", repo.split('/').next().unwrap_or(repo), d.head_ref)
        }
        _ => d.head_ref.clone(),
    };
    let segs = vec![
        author,
        Seg::new(" wants to merge into ", Role::Meta),
        chip(&d.base_ref, Bg::PrimaryContainer),
        Seg::new(" from ", Role::Meta),
        chip(head, Bg::PrimaryContainer),
    ];
    page.wrapped(segs, 0, Frame::None);
    let mut meta = Vec::from(changes(s.additions, s.deletions));
    meta.push(Seg::new(
        format!(
            " · opened {} · updated {}",
            time::ago_iso(&d.created_at, now),
            time::ago_iso(&s.updated_at, now)
        ),
        Role::Meta,
    ));
    labels(&mut meta, &d.labels);
    page.wrapped(meta, 0, Frame::None);
    page.rule(0, Frame::None);
}

/// GitHub's merge box: checks, reviews, conflicts.
fn merge_box(page: &mut Page, pr: &PrRef, d: &PrDetail, icons: Icons) {
    let s = &d.summary;
    if matches!(s.state, PrState::Merged | PrState::Closed) {
        return;
    }
    connector(page);
    let (headline, role) = match (s.checks, d.mergeable) {
        (_, Mergeable::Conflicting) => (
            "This branch has conflicts that must be resolved",
            Role::Error,
        ),
        (Some(ChecksState::Failing), _) => ("Some checks were not successful", Role::Error),
        (Some(ChecksState::Pending), _) => ("Some checks haven't completed yet", Role::Accent),
        _ => match s.review {
            Some(ReviewDecision::ChangesRequested) => ("Changes requested", Role::Error),
            Some(ReviewDecision::ReviewRequired) => ("Review required", Role::Accent),
            _ => ("This branch can be merged", Role::Success),
        },
    };
    page.box_top(vec![Seg::new(headline, role)], Vec::new());
    // Each row: passed, failed, or neither yet.
    let review = s.review.map(|r| match r {
        ReviewDecision::Approved => (Some(true), "Changes approved"),
        ReviewDecision::ChangesRequested => (Some(false), "Changes requested"),
        ReviewDecision::ReviewRequired => (None, "Review required"),
    });
    let checks = s.checks.map(|c| match c {
        ChecksState::Passing => (Some(true), "All checks have passed"),
        ChecksState::Failing => (Some(false), "Some checks were not successful"),
        ChecksState::Pending => (None, "Some checks haven't completed yet"),
    });
    let conflicts = match d.mergeable {
        Mergeable::Yes => (Some(true), "No conflicts with the base branch"),
        Mergeable::Conflicting => (Some(false), "This branch has conflicts"),
        Mergeable::Unknown => (None, "Checking for conflicts…"),
    };
    for (ok, text) in [review, checks, Some(conflicts)].into_iter().flatten() {
        let (icon, role) = match ok {
            Some(true) => ("✓", Role::Success),
            Some(false) => ("✗", Role::Error),
            None => ("●", Role::Accent),
        };
        body(
            page,
            vec![
                Seg::new(format!("{icon}  "), role),
                Seg::new(text, Role::Body),
            ],
        );
    }
    page.box_rule();
    item(page, url::pull_tab(pr, "files"), |page, link| {
        let review = format!("Review the {} changed files", d.changed_files);
        body(
            page,
            vec![
                Seg::new(format!("{}  ", icons.external()), Role::Meta),
                Seg::linked(review, Role::Link, link),
            ],
        );
    });
    page.box_bottom();
}

pub fn pr_conversation(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Option<&PrActivity>,
    aside: Option<u16>,
    cx: PageCtx<'_>,
) {
    // Comments and reviews, merged in time order below.
    enum Entry<'a> {
        Comment(&'a Comment),
        Review(&'a ghtui_api::browse::ReviewSummary),
    }
    let PageCtx { icons, keys, now } = cx;
    pr_summary(page, pr, d, now);
    let op = &d.summary.author;
    let talk = Conversation {
        base: LinkBase::new(&pr.repo, &d.head_oid, ""),
        op,
        now,
    };
    talk.said(page, op, "opened", &d.created_at, &d.body, true);
    let Some(a) = activity else {
        connector(page);
        page.line(vec![Seg::new("   Loading the conversation…", Role::Meta)]);
        return;
    };
    let mut entries: Vec<(&str, Entry<'_>)> = a
        .comments
        .iter()
        .map(|c| (c.created_at.as_str(), Entry::Comment(c)))
        .chain(
            a.reviews
                .iter()
                .map(|r| (r.submitted_at.as_str(), Entry::Review(r))),
        )
        .collect();
    entries.sort_by(|x, y| x.0.cmp(y.0));
    for (_, entry) in entries {
        connector(page);
        match entry {
            Entry::Comment(c) => talk.comment(page, c),
            Entry::Review(r) => {
                let (icon, what) = match r.state.as_str() {
                    "approved" => (("✓", Role::Success), "approved these changes"),
                    "requested changes" => (("✗", Role::Error), "requested changes"),
                    "dismissed" => (("○", Role::Meta), "had a review dismissed"),
                    _ => (("◉", Role::Meta), "reviewed"),
                };
                talk.event(page, icon, &r.author, what, &r.submitted_at);
                if !r.body.trim().is_empty() {
                    connector(page);
                    talk.said(
                        page,
                        &r.author,
                        "commented",
                        &r.submitted_at,
                        &r.body,
                        false,
                    );
                }
            }
        }
    }
    merge_box(page, pr, d, icons);
    add_comment(page, keys);
    if let Some(width) = aside {
        let reviewers = unique(a.reviews.iter().map(|r| &r.author));
        page.build_aside(width, |side| {
            people(side, "Reviewers", &reviewers, "No reviews");
            label_list(side, &d.labels);
            aside_heading(side, "Size");
            let mut size = Vec::from(changes(d.summary.additions, d.summary.deletions));
            size.push(Seg::new(
                format!(" in {}", plural(d.changed_files, "file")),
                Role::Meta,
            ));
            side.line(size);
        });
    }
}

/// A pull request's Checks tab: the checks on its head.
pub fn pr_checks(page: &mut Page, pr: &PrRef, d: &PrDetail, c: Option<&Checks>, now: u64) {
    pr_summary(page, pr, d, now);
    page.blank();
    checks(page, c, now);
}

/// A repository's Actions tab: the checks on its default branch.
pub fn actions(page: &mut Page, branch: Option<&str>, c: Option<&Checks>, now: u64) {
    let mut title = vec![Seg::new("Actions", Role::Title)];
    if let Some(branch) = branch {
        title.push(Seg::new(format!("  checks on {branch}"), Role::Meta));
    }
    page.wrapped(title, 0, Frame::None);
    page.blank();
    checks(page, c, now);
}

pub fn pr_commits(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Option<&PrActivity>,
    now: u64,
) {
    pr_summary(page, pr, d, now);
    let Some(a) = activity else {
        page.blank();
        page.line(vec![Seg::new("Loading commits…", Role::Meta)]);
        return;
    };
    commit_rows(page, &pr.repo, &a.commits, None, now);
}

/// Commits in boxes by day, each linked to its page; `more` adds a "Load
/// more" row (shown so far, total).
fn commit_rows(
    page: &mut Page,
    repo: &RepoId,
    commits: &[CommitInfo],
    more: Option<(usize, u64)>,
    now: u64,
) {
    let mut current_day = String::new();
    for c in commits {
        let date = day(&c.date);
        if date == current_day {
            page.box_rule();
        } else {
            if !current_day.is_empty() {
                page.box_bottom();
            }
            page.box_top(
                vec![Seg::new(format!("◷ Commits on {date}"), Role::Meta)],
                Vec::new(),
            );
            current_day = date;
        }
        item(page, url::commit(repo, &c.oid), |page, link| {
            page.box_line(
                vec![Seg::linked(c.headline.clone(), Role::Strong, link)],
                vec![Seg::new(crate::text::short_sha(&c.oid), Role::Code)],
                0,
            );
            let committed = format!("{} committed {}", c.author, time::ago_iso(&c.date, now));
            body(page, vec![Seg::new(committed, Role::Meta)]);
        });
    }
    if let Some((shown, total)) = more {
        more_row(page, true, shown, total);
    }
    if !current_day.is_empty() {
        page.box_bottom();
    }
}

/// A revision's commits (of `path`, if it isn't empty), newest first.
pub fn commit_history(
    page: &mut Page,
    repo: &RepoId,
    rev: &str,
    path: &str,
    history: Option<&Results<CommitInfo>>,
    now: u64,
) {
    let mut title = vec![Seg::new("Commits", Role::Title)];
    let at = if path.is_empty() {
        format!("  {rev}")
    } else {
        format!("  {rev} · {path}")
    };
    title.push(Seg::new(at, Role::Meta));
    page.wrapped(title, 0, Frame::None);
    page.blank();
    let Some(h) = history else {
        page.line(vec![Seg::new("Loading commits…", Role::Meta)]);
        return;
    };
    if h.items.is_empty() {
        empty_box(page, Seg::new("◷ Commits", Role::Meta), "No commits here.");
        return;
    }
    let more = h.next.is_some().then_some((h.items.len(), h.total));
    commit_rows(page, repo, &h.items, more, now);
}

// ---- checks --------------------------------------------------------------------------------

fn outcome_mark(o: CheckOutcome) -> (&'static str, Role) {
    match o {
        CheckOutcome::Success => ("✓", Role::Success),
        CheckOutcome::Failure => ("✗", Role::Error),
        CheckOutcome::Pending => ("●", Role::Accent),
        CheckOutcome::Cancelled | CheckOutcome::Skipped => ("⊘", Role::Meta),
        CheckOutcome::Neutral => ("◦", Role::Meta),
    }
}

/// The checks on a commit, grouped by workflow or app, failures first.
/// Each links to its details on GitHub (logs, re-runs).
pub fn checks(page: &mut Page, checks: Option<&Checks>, now: u64) {
    let Some(c) = checks else {
        page.line(vec![Seg::new("Loading checks…", Role::Meta)]);
        return;
    };
    let sha = crate::text::short_sha(&c.oid);
    if c.items.is_empty() {
        let title = Seg::new(format!("✓ Checks on {sha}"), Role::Meta);
        empty_box(page, title, "No checks ran on this commit.");
        return;
    }
    let mut summary = Vec::new();
    for (outcome, word) in [
        (CheckOutcome::Failure, "failed"),
        (CheckOutcome::Pending, "running"),
        (CheckOutcome::Success, "passed"),
        (CheckOutcome::Cancelled, "cancelled"),
        (CheckOutcome::Skipped, "skipped"),
        (CheckOutcome::Neutral, "neutral"),
    ] {
        let n = c.items.iter().filter(|i| i.outcome == outcome).count();
        if n > 0 {
            if !summary.is_empty() {
                summary.push(Seg::new(" · ", Role::Meta));
            }
            let (mark, role) = outcome_mark(outcome);
            summary.push(Seg::new(format!("{mark} {n} {word}"), role));
        }
    }
    summary.push(Seg::new(format!("   on {sha}"), Role::Meta));
    let shown = c.items.len() as u64;
    if c.total > shown {
        summary.push(Seg::new(
            format!(" · {shown} of {} shown", c.total),
            Role::Meta,
        ));
    }
    page.wrapped(summary, 0, Frame::None);
    page.blank();
    // Groups in order of their worst check, then by name.
    let mut groups: Vec<(&str, Vec<&CheckItem>)> = Vec::new();
    for check in &c.items {
        match groups.iter_mut().find(|(g, _)| *g == check.group) {
            Some((_, checks)) => checks.push(check),
            None => groups.push((&check.group, vec![check])),
        }
    }
    for (_, checks) in &mut groups {
        checks.sort_by(|a, b| (a.outcome, &a.name).cmp(&(b.outcome, &b.name)));
    }
    groups.sort_by_key(|(g, checks)| (checks.first().map(|c| c.outcome), *g));
    for (group, checks) in groups {
        page.box_top(vec![Seg::new(group.to_owned(), Role::Meta)], Vec::new());
        for (n, check) in checks.into_iter().enumerate() {
            if n > 0 {
                page.box_rule();
            }
            check_row(page, check, now);
        }
        page.box_bottom();
        page.blank();
    }
}

fn check_row(page: &mut Page, check: &CheckItem, now: u64) {
    let (mark, role) = outcome_mark(check.outcome);
    let took = match (&check.started_at, &check.completed_at, check.outcome) {
        (Some(start), Some(end), _) => time::duration_iso(start, end),
        (Some(start), None, CheckOutcome::Pending) => {
            Some(format!("started {}", time::ago_iso(start, now)))
        }
        _ => None,
    };
    let right = took.map_or_else(Vec::new, |t| vec![Seg::new(t, Role::Meta)]);
    let row = |page: &mut Page, link: Option<u32>| {
        let name = match link {
            Some(link) => Seg::linked(check.name.clone(), Role::Strong, link),
            None => Seg::new(check.name.clone(), Role::Strong),
        };
        page.box_line(vec![Seg::new(format!("{mark} "), role), name], right, 0);
        if let Some(summary) = check.summary.as_ref().filter(|_| !page.compact) {
            body(page, vec![Seg::new(format!("  {summary}"), Role::Meta)]);
        }
    };
    match &check.url {
        Some(url) => item(page, url.clone(), |page, link| row(page, Some(link))),
        None => row(page, None),
    }
}

// ---- actions ----------------------------------------------------------------------------------

/// How long something took, or how long ago it started while it runs.
fn took(
    start: Option<&String>,
    end: Option<&String>,
    outcome: CheckOutcome,
    now: u64,
) -> Option<String> {
    match (start, end, outcome) {
        (Some(s), Some(e), _) => time::duration_iso(s, e),
        (Some(s), None, CheckOutcome::Pending) => {
            Some(format!("started {}", time::ago_iso(s, now)))
        }
        _ => None,
    }
}

fn outcome_title(page: &mut Page, title: String, outcome: CheckOutcome) {
    let (mark, role) = outcome_mark(outcome);
    page.wrapped(
        vec![
            Seg::new(format!("{mark} "), role),
            Seg::new(title, Role::Title),
        ],
        0,
        Frame::None,
    );
}

/// A workflow run: what it ran for, and its jobs, failures first. Re-runs
/// and cancelling write, so they stay on GitHub.
pub fn workflow_run(page: &mut Page, repo: &RepoId, run: &WorkflowRun, now: u64) {
    outcome_title(page, format!("{} #{}", run.name, run.number), run.outcome);
    if !run.title.is_empty() {
        page.wrapped(
            vec![Seg::new(run.title.clone(), Role::Body)],
            0,
            Frame::None,
        );
    }
    let mut meta = vec![Seg::new(format!("{} · ", run.event), Role::Meta)];
    if let Some(branch) = &run.branch {
        meta.push(link_seg(
            page,
            branch.clone(),
            url::tree(repo, branch, ""),
            Role::Code,
        ));
        meta.push(Seg::new(" · ", Role::Meta));
    }
    let sha = crate::text::short_sha(&run.sha);
    meta.push(link_seg(page, sha, url::commit(repo, &run.sha), Role::Code));
    if let Some(actor) = &run.actor {
        meta.push(Seg::new(" · ", Role::Meta));
        meta.push(link_seg(
            page,
            actor.clone(),
            url::user(actor),
            Role::Strong,
        ));
    }
    if let Some(at) = &run.started_at {
        meta.push(Seg::new(
            format!(" · started {}", time::ago_iso(at, now)),
            Role::Meta,
        ));
    }
    if let Some(d) = took(
        run.started_at.as_ref(),
        run.updated_at.as_ref(),
        run.outcome,
        now,
    )
    .filter(|_| run.outcome != CheckOutcome::Pending)
    {
        meta.push(Seg::new(format!(" · took {d}"), Role::Meta));
    }
    page.wrapped(meta, 0, Frame::None);
    let file = run.path.rsplit('/').next().unwrap_or(&run.path).to_owned();
    let mut facts = Vec::new();
    if !file.is_empty() {
        let workflow = format!(
            "{}/actions/workflows/{}",
            url::repo(repo),
            url::encode_path(&file)
        );
        facts.push(Seg::new("Workflow ", Role::Meta));
        facts.push(link_seg(page, file, workflow, Role::Link));
        facts.push(Seg::new("   ", Role::Meta));
    }
    if run.attempt > 1 {
        facts.push(Seg::new(format!("Attempt {}   ", run.attempt), Role::Meta));
    }
    facts.push(Seg::new(
        "Re-running and cancelling are on GitHub (o)",
        Role::Meta,
    ));
    page.wrapped(facts, 0, Frame::None);
    page.blank();
    let mut jobs: Vec<&JobSummary> = run.jobs.iter().collect();
    jobs.sort_by(|a, b| (a.outcome, &a.name).cmp(&(b.outcome, &b.name)));
    let title = vec![Seg::new(format!("Jobs  {}", jobs.len()), Role::Strong)];
    list_box(page, title, Vec::new(), &jobs, "No jobs.", |page, job| {
        let target = format!("{}/actions/runs/{}/job/{}", url::repo(repo), run.id, job.id);
        item(page, target, |page, link| {
            let (mark, role) = outcome_mark(job.outcome);
            let right = took(
                job.started_at.as_ref(),
                job.completed_at.as_ref(),
                job.outcome,
                now,
            )
            .map_or_else(Vec::new, |t| vec![Seg::new(t, Role::Meta)]);
            let segs = vec![
                Seg::new(format!("{mark} "), role),
                Seg::linked(job.name.clone(), Role::Strong, link),
            ];
            page.box_line(segs, right, 0);
        });
    });
}

/// A log line without its timestamp, and when it was written (ISO 8601).
fn log_line(line: &str) -> (Option<&str>, &str) {
    match line.split_once(' ') {
        Some((at, rest))
            if at.len() >= 20 && at.ends_with('Z') && at.as_bytes().get(4) == Some(&b'-') =>
        {
            (Some(at), rest)
        }
        _ => (None, line),
    }
}

/// Which step each log line belongs to. GitHub's one log for a job
/// doesn't say, and its steps' times are to the second, so a step that
/// ran begins at the line that starts it (`##[group]Run …`, `Post job
/// cleanup.`, `Cleaning up orphan processes`) or once the step before
/// has ended, whichever comes first.
fn step_lines<'a>(job: &Job, log: &'a str) -> Vec<(u32, Vec<&'a str>)> {
    let mut out: Vec<(u32, Vec<&str>)> = job.steps.iter().map(|s| (s.number, Vec::new())).collect();
    let second = |at: Option<&String>| at.map(|at| at.get(..19).unwrap_or(at).to_owned());
    // Skipped steps log nothing.
    let ran: Vec<usize> = job
        .steps
        .iter()
        .enumerate()
        .filter(|(_, s)| s.outcome != CheckOutcome::Skipped)
        .map(|(i, _)| i)
        .collect();
    let mut at = 0;
    for raw in log.lines() {
        let (time, text) = log_line(raw.trim_start_matches('\u{feff}'));
        if let Some(time) = time {
            let t = time.get(..19).unwrap_or(time);
            let starts = text.starts_with("##[group]Run ")
                || text == "Post job cleanup."
                || text == "Cleaning up orphan processes";
            while let (Some(current), Some(next)) = (
                ran.get(at).and_then(|&i| job.steps.get(i)),
                ran.get(at + 1).and_then(|&i| job.steps.get(i)),
            ) {
                let next_started =
                    second(next.started_at.as_ref()).is_some_and(|s| s.as_str() <= t);
                let ended = second(current.completed_at.as_ref()).is_some_and(|s| s.as_str() < t);
                if !next_started || !(ended || starts) {
                    break;
                }
                at += 1;
                // A start line starts one step.
                if starts && !ended {
                    break;
                }
            }
        }
        if let Some((_, lines)) = ran.get(at).and_then(|&i| out.get_mut(i)) {
            lines.push(text);
        }
    }
    out
}

/// `text` without terminal escape sequences (colors, mostly).
fn strip_escapes(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: parameters, then a final byte from @ to ~.
            Some('[') => {
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: up to BEL or ESC \.
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\u{7}' || (c == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A log line as shown: group markers and errors stand out.
fn log_seg(text: &str) -> Seg {
    let text = &strip_escapes(text);
    if let Some(group) = text.strip_prefix("##[group]") {
        Seg::new(format!("▸ {group}"), Role::Strong)
    } else if text.starts_with("##[endgroup]") {
        Seg::new("", Role::Meta)
    } else if let Some(error) = text.strip_prefix("##[error]") {
        Seg::new(error.to_owned(), Role::Removed)
    } else if let Some(warning) = text.strip_prefix("##[warning]") {
        Seg::new(warning.to_owned(), Role::Accent)
    } else {
        Seg::new(text.to_owned(), Role::Body)
    }
}

/// How many lines of a step's log show.
const STEP_LINES: usize = 400;

/// A job: its steps, each collapsed to its outcome, failing ones (and the
/// one a link points at) expanded with their log. With `query`, only the
/// log lines that contain it, under their steps.
pub fn job(page: &mut Page, repo: &RepoId, job: &Job, log: Option<&str>, at: JobAt<'_>, now: u64) {
    outcome_title(page, job.name.clone(), job.outcome);
    let mut meta = Vec::new();
    let run = format!("{}/actions/runs/{}", url::repo(repo), job.run_id);
    meta.push(link_seg(
        page,
        format!("Run {}", job.run_id),
        run,
        Role::Link,
    ));
    if let Some(d) = took(
        job.started_at.as_ref(),
        job.completed_at.as_ref(),
        job.outcome,
        now,
    ) {
        meta.push(Seg::new(format!(" · {d}"), Role::Meta));
    }
    page.wrapped(meta, 0, Frame::None);
    page.blank();
    if !at.query.is_empty() {
        filter_field(page, at.query, at.keys);
        page.blank();
    }
    let lines = log.map(|log| step_lines(job, log));
    let title = vec![Seg::new(
        format!("Steps  {}", job.steps.len()),
        Role::Strong,
    )];
    let right = vec![Seg::new(
        format!("{} filters the log", at.keys.filter),
        Role::Meta,
    )];
    page.box_top(title, right);
    for (n, step) in job.steps.iter().enumerate() {
        if n > 0 {
            page.box_rule();
        }
        let target = format!(
            "{}/actions/runs/{}/job/{}#step:{}:1",
            url::repo(repo),
            job.run_id,
            job.id,
            step.number
        );
        item(page, target, |page, link| {
            let (mark, role) = outcome_mark(step.outcome);
            let right = took(
                step.started_at.as_ref(),
                step.completed_at.as_ref(),
                step.outcome,
                now,
            )
            .map_or_else(Vec::new, |t| vec![Seg::new(t, Role::Meta)]);
            let segs = vec![
                Seg::new(format!("{mark} "), role),
                Seg::linked(step.name.clone(), Role::Strong, link),
            ];
            page.box_line(segs, right, 0);
        });
        let pointed = at.step.map(|(s, _)| s) == Some(step.number);
        let open = pointed || step.outcome == CheckOutcome::Failure || !at.query.is_empty();
        if !open {
            continue;
        }
        let Some(lines) = &lines else {
            body(page, vec![Seg::new("Loading the log…", Role::Meta)]);
            continue;
        };
        let step_lines: Vec<(usize, &str)> = lines
            .iter()
            .find(|(n, _)| *n == step.number)
            .map(|(_, l)| l.iter().copied().enumerate().collect())
            .unwrap_or_default();
        let shown: Vec<(usize, &str)> = if at.query.is_empty() {
            step_lines
        } else {
            let q = at.query.to_lowercase();
            step_lines
                .into_iter()
                .filter(|(_, l)| l.to_lowercase().contains(&q))
                .collect()
        };
        let skip = shown.len().saturating_sub(STEP_LINES);
        if skip > 0 {
            body(
                page,
                vec![Seg::new(
                    format!("… {skip} earlier lines (o shows them all on GitHub)"),
                    Role::Meta,
                )],
            );
        }
        let width = shown.last().map_or(1, |(i, _)| (i + 1).to_string().len());
        for (i, text) in shown.into_iter().skip(skip) {
            if pointed && at.step.map(|(_, l)| l as usize) == Some(i + 1) && page.jump.is_none() {
                page.jump = Some(page.lines.len());
            }
            let number = Seg::new(
                format!("{:>width$}  ", i + 1),
                Role::Syntax(Syntax::Comment),
            );
            page.push(PageLine {
                segs: vec![number, log_seg(text)],
                frame: Frame::Body,
                tone: Tone::Code,
                ..PageLine::default()
            });
        }
        if pointed && page.jump.is_none() {
            page.jump = Some(page.lines.len().saturating_sub(1));
        }
    }
    page.box_bottom();
}

/// Where a job page is: the step a link points at, the log's filter.
#[derive(Clone, Copy)]
pub struct JobAt<'a> {
    pub step: Option<(u32, u32)>,
    pub query: &'a str,
    pub keys: Keys<'a>,
}

/// A workflow and its runs, newest first.
pub fn workflow(
    page: &mut Page,
    repo: &RepoId,
    wf: Option<&Workflow>,
    runs: Option<&Results<RunSummary>>,
    now: u64,
) {
    if let Some(wf) = wf {
        let mut title = vec![Seg::new(wf.name.clone(), Role::Title)];
        if wf.state != "active" {
            title.push(space());
            title.push(chip(wf.state.replace('_', " "), Bg::SecondaryContainer));
        }
        page.wrapped(title, 0, Frame::None);
        let file = link_seg(
            page,
            wf.path.clone(),
            url::blob(repo, "HEAD", &wf.path),
            Role::Code,
        );
        page.wrapped(vec![file], 0, Frame::None);
        page.blank();
    }
    let Some(r) = runs else {
        page.line(vec![Seg::new("Loading runs…", Role::Meta)]);
        return;
    };
    let title = vec![Seg::new(
        format!("Runs  {}", compact(r.total)),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    if r.items.is_empty() {
        empty_row(page, "No runs yet.");
    }
    box_rows(page, &r.items, |page, run| {
        item(
            page,
            format!("{}/actions/runs/{}", url::repo(repo), run.id),
            |page, link| {
                let (mark, role) = outcome_mark(run.outcome);
                let title = if run.title.is_empty() {
                    format!("Run #{}", run.number)
                } else {
                    run.title.clone()
                };
                page.box_line(
                    vec![
                        Seg::new(format!("{mark} "), role),
                        Seg::linked(title, Role::Strong, link),
                    ],
                    Vec::new(),
                    0,
                );
                let mut meta = format!("#{} · {}", run.number, run.event);
                if let Some(b) = &run.branch {
                    meta.push_str(&format!(" · {b}"));
                }
                if let Some(a) = &run.actor {
                    meta.push_str(&format!(" · {a}"));
                }
                if let Some(at) = &run.created_at {
                    meta.push_str(&format!(" · {}", time::ago_iso(at, now)));
                }
                body(page, vec![Seg::new(meta, Role::Meta)]);
            },
        );
    });
    more_row(page, r.next.is_some(), r.items.len(), r.total);
    page.box_bottom();
}

/// A commit's checks.
pub fn commit_checks(page: &mut Page, c: Option<&Checks>, now: u64) {
    checks(page, c, now);
}

// ---- discussions ------------------------------------------------------------------------------

/// A list of discussions: the categories to filter by (`base` is the
/// list's URL), then the discussions, most recently updated first.
pub fn discussions(
    page: &mut Page,
    base: &str,
    list: Option<&DiscussionList>,
    category: Option<&str>,
    now: u64,
) {
    let Some(l) = list else {
        page.line(vec![Seg::new("Loading discussions…", Role::Meta)]);
        return;
    };
    if !l.categories.is_empty() {
        let mut segs = Vec::new();
        let all = category.is_none();
        segs.push(link_seg(
            page,
            "All",
            base.to_owned(),
            if all { Role::Strong } else { Role::Link },
        ));
        for c in &l.categories {
            segs.push(Seg::new(" · ", Role::Meta));
            let target = format!("{base}/categories/{}", c.slug);
            let role = if category == Some(c.slug.as_str()) {
                Role::Strong
            } else {
                Role::Link
            };
            segs.push(link_seg(page, c.name.clone(), target, role));
        }
        page.wrapped(segs, 0, Frame::None);
        page.blank();
    }
    let r = &l.results;
    let title = vec![Seg::new(
        format!("Discussions  {}", compact(r.total)),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    if r.items.is_empty() {
        empty_row(page, "No discussions yet.");
    }
    box_rows(page, &r.items, |page, d| {
        item(page, format!("{base}/{}", d.number), |page, link| {
            let mut segs = vec![Seg::linked(d.title.clone(), Role::Strong, link)];
            if d.answered {
                segs.push(space());
                segs.push(chip("✓ Answered", Bg::SuccessContainer));
            }
            body(page, segs);
            let meta = format!(
                "#{} · {} · {} · {} · ▲ {} · updated {}",
                d.number,
                d.category,
                d.author,
                plural(d.comments, "comment"),
                d.upvotes,
                time::ago_iso(&d.updated_at, now)
            );
            body(page, vec![Seg::new(meta, Role::Meta)]);
        });
    });
    more_row(page, r.next.is_some(), r.items.len(), r.total);
    page.box_bottom();
}

/// A discussion: its post, then its comments (the answer marked) with their
/// replies, each named for links to it (`#discussioncomment-…`).
pub fn discussion(page: &mut Page, d: &DiscussionDetail, now: u64) {
    page.wrapped(
        vec![
            Seg::new(d.title.clone(), Role::Title),
            Seg::new(format!("  #{}", d.number), Role::Meta),
        ],
        0,
        Frame::None,
    );
    let mut segs = vec![chip(d.category.clone(), Bg::SecondaryContainer)];
    if d.answered {
        segs.push(space());
        segs.push(chip("✓ Answered", Bg::SuccessContainer));
    }
    segs.push(Seg::new(
        format!(
            "  ▲ {} · {}",
            d.upvotes,
            plural(d.comments.len() as u64, "comment")
        ),
        Role::Meta,
    ));
    page.wrapped(segs, 0, Frame::None);
    page.rule(0, Frame::None);
    let talk = Conversation {
        base: LinkBase::new(&d.repo, "HEAD", ""),
        op: &d.author,
        now,
    };
    talk.said(page, &d.author, "started", &d.created_at, &d.body, true);
    let anchor = |c: &Comment| c.id.map(|id| format!("discussioncomment-{id}"));
    for c in &d.comments {
        connector(page);
        let badge = if c.answer {
            Some(chip("✓ Answer", Bg::SuccessContainer))
        } else {
            (c.comment.author == d.author).then(|| chip("Author", Bg::SecondaryContainer))
        };
        let verb = format!("commented · ▲ {}", c.upvotes);
        let comment = &c.comment;
        talk.said_at(
            page,
            anchor(comment),
            &comment.author,
            &verb,
            &comment.created_at,
            &comment.body,
            badge,
        );
        for r in &c.replies {
            let badge = (r.author == d.author).then(|| chip("Author", Bg::SecondaryContainer));
            talk.said_at(
                page,
                anchor(r),
                &r.author,
                "replied",
                &r.created_at,
                &r.body,
                badge,
            );
        }
    }
}

// ---- releases and tags ---------------------------------------------------------------------

fn release_chips(segs: &mut Vec<Seg>, r: &Release) {
    for (on, text, bg) in [
        (r.latest, "Latest", Bg::SuccessContainer),
        (r.prerelease, "Pre-release", Bg::TertiaryContainer),
        (r.draft, "Draft", Bg::SecondaryContainer),
    ] {
        if on {
            segs.push(space());
            segs.push(chip(text, bg));
        }
    }
}

/// Who released it and when.
fn release_meta(r: &Release, now: u64) -> String {
    let mut meta = r.tag.clone();
    if let Some(author) = &r.author {
        meta.push_str(&format!(" · {author}"));
    }
    match &r.published_at {
        Some(at) => meta.push_str(&format!(" released {}", time::ago_iso(at, now))),
        None => meta.push_str(" · not published"),
    }
    meta
}

/// A repository's releases, newest first, each linked to its page.
pub fn releases(page: &mut Page, repo: &RepoId, list: Option<&Results<Release>>, now: u64) {
    let Some(l) = list else {
        page.line(vec![Seg::new("Loading releases…", Role::Meta)]);
        return;
    };
    let title = format!("Releases  {}", compact(l.total));
    page.box_top(vec![Seg::new(title, Role::Strong)], Vec::new());
    if l.items.is_empty() {
        empty_row(page, "No releases yet.");
    }
    box_rows(page, &l.items, |page, r| {
        item(page, url::release(repo, &r.tag), |page, link| {
            let mut segs = vec![Seg::linked(r.name.clone(), Role::Link, link)];
            release_chips(&mut segs, r);
            body(page, segs);
            body(page, vec![Seg::new(release_meta(r, now), Role::Meta)]);
        });
    });
    more_row(page, l.next.is_some(), l.items.len(), l.total);
    page.box_bottom();
}

/// A release: its notes and its assets (downloads stay on GitHub).
pub fn release(page: &mut Page, repo: &RepoId, r: &Release, now: u64) {
    let mut title = vec![Seg::new(r.name.clone(), Role::Title)];
    release_chips(&mut title, r);
    page.wrapped(title, 0, Frame::None);
    let tag = link_seg(page, r.tag.clone(), url::tree(repo, &r.tag, ""), Role::Code);
    let mut meta = vec![Seg::new("◇ ", Role::Meta), tag];
    if let Some(author) = &r.author {
        meta.push(Seg::new(" · ", Role::Meta));
        meta.push(link_seg(
            page,
            author.clone(),
            url::user(author),
            Role::Strong,
        ));
    }
    if let Some(at) = &r.published_at {
        meta.push(Seg::new(
            format!(" released {}", time::ago_iso(at, now)),
            Role::Meta,
        ));
    }
    page.wrapped(meta, 0, Frame::None);
    page.blank();
    match &r.notes {
        Some(notes) => {
            let base = LinkBase::new(repo, &r.tag, "");
            markdown::render(page, notes, Some(&base), Frame::None);
        }
        None => page.line(vec![Seg::new("No release notes.", Role::Meta)]),
    }
    page.blank();
    let title = Seg::new(format!("Assets  {}", r.assets.len()), Role::Strong);
    if r.assets.is_empty() {
        empty_box(page, title, "No assets.");
        return;
    }
    page.box_top(vec![title], Vec::new());
    box_rows(page, &r.assets, |page, a| {
        item(page, a.url.clone(), |page, link| {
            let right = vec![Seg::new(
                format!(
                    "{} · {} downloads",
                    crate::text::size(a.size),
                    compact(a.downloads)
                ),
                Role::Meta,
            )];
            page.box_line(
                vec![Seg::linked(a.name.clone(), Role::Link, link)],
                right,
                0,
            );
        });
    });
    page.box_bottom();
}

/// A repository's tags, each linked to its code.
pub fn tags(page: &mut Page, repo: &RepoId, list: Option<&Results<TagInfo>>, now: u64) {
    let Some(l) = list else {
        page.line(vec![Seg::new("Loading tags…", Role::Meta)]);
        return;
    };
    let title = format!("Tags  {}", compact(l.total));
    page.box_top(vec![Seg::new(title, Role::Strong)], Vec::new());
    if l.items.is_empty() {
        empty_row(page, "No tags yet.");
    }
    box_rows(page, &l.items, |page, t| {
        item(page, url::tree(repo, &t.name, ""), |page, link| {
            let mut right = Vec::new();
            if let Some(date) = &t.date {
                right.push(Seg::new(time::ago_iso(date, now), Role::Meta));
            }
            if let Some(oid) = &t.oid {
                right.push(Seg::new(
                    format!("  {}", crate::text::short_sha(oid)),
                    Role::Code,
                ));
            }
            page.box_line(
                vec![Seg::linked(t.name.clone(), Role::Link, link)],
                right,
                0,
            );
        });
    });
    more_row(page, l.next.is_some(), l.items.len(), l.total);
    page.box_bottom();
}

/// A repository's branches by name: each one's latest commit and pull
/// request.
pub fn branches(
    page: &mut Page,
    repo: &RepoId,
    list: Option<&Results<BranchInfo>>,
    icons: Icons,
    now: u64,
) {
    let Some(l) = list else {
        page.line(vec![Seg::new("Loading branches…", Role::Meta)]);
        return;
    };
    let title = format!("Branches  {}", compact(l.total));
    page.box_top(vec![Seg::new(title, Role::Strong)], Vec::new());
    if l.items.is_empty() {
        empty_row(page, "No branches.");
    }
    box_rows(page, &l.items, |page, b| {
        item(page, url::tree(repo, &b.name, ""), |page, link| {
            let mut segs = vec![Seg::linked(b.name.clone(), Role::Link, link)];
            if b.default {
                segs.push(space());
                segs.push(chip("default", Bg::SecondaryContainer));
            }
            let mut right = Vec::new();
            if let Some((number, state)) = b.pr {
                let (icon, role) = issue_icon(icons, state, true);
                right.push(Seg::new(format!("{icon} "), role));
                right.push(link_seg(
                    page,
                    format!("#{number}"),
                    url::pull(&PrRef {
                        repo: repo.clone(),
                        number,
                    }),
                    Role::Link,
                ));
                right.push(Seg::new("  ", Role::Meta));
            }
            if let Some(date) = &b.date {
                right.push(Seg::new(time::ago_iso(date, now), Role::Meta));
            }
            page.box_line(segs, right, 0);
            let mut meta = Vec::new();
            if let Some(oid) = &b.oid {
                let sha = crate::text::short_sha(oid);
                meta.push(link_seg(page, sha, url::commit(repo, oid), Role::Code));
                meta.push(Seg::new(" ", Role::Meta));
            }
            let what = [b.headline.as_deref(), b.author.as_deref()];
            let text = what.into_iter().flatten().collect::<Vec<_>>().join(" · ");
            meta.push(Seg::new(text, Role::Meta));
            body(page, meta);
        });
    });
    more_row(page, l.next.is_some(), l.items.len(), l.total);
    page.box_bottom();
}
// ---- comparisons ------------------------------------------------------------------------------

/// Two revisions compared: how they differ, a link to the files changed,
/// and the commits from one to the other, oldest first.
pub fn compare(page: &mut Page, repo: &RepoId, spec: &str, c: &Comparison, now: u64) {
    let (base, head) = spec
        .split_once("...")
        .or_else(|| spec.split_once(".."))
        .unwrap_or((spec, ""));
    page.wrapped(
        vec![
            Seg::new("Comparing ", Role::Title),
            Seg::new(base.to_owned(), Role::Code),
            Seg::new(if spec.contains("...") { "..." } else { ".." }, Role::Meta),
            Seg::new(head.to_owned(), Role::Code),
        ],
        0,
        Frame::None,
    );
    let status = match c.status.as_str() {
        "identical" => "These are identical.".to_owned(),
        "behind" => format!("{head} is {} behind {base}.", plural(c.behind, "commit")),
        "ahead" => format!("{head} is {} ahead of {base}.", plural(c.ahead, "commit")),
        _ => format!(
            "{head} is {} ahead of and {} behind {base}.",
            plural(c.ahead, "commit"),
            plural(c.behind, "commit")
        ),
    };
    page.wrapped(vec![Seg::new(status, Role::Meta)], 0, Frame::None);
    let files = format!("{}#files", url::compare(repo, &c.from, &c.to));
    let changed = if c.files == 1 {
        "1 file changed".to_owned()
    } else {
        format!("{} files changed", c.files)
    };
    let mut segs = vec![link_seg(page, format!("± {changed}"), files, Role::Link)];
    segs.push(Seg::new("  ", Role::Meta));
    segs.extend(changes(c.additions, c.deletions));
    page.wrapped(segs, 0, Frame::None);
    page.blank();
    if c.commits.is_empty() {
        empty_box(
            page,
            Seg::new("◷ Commits", Role::Meta),
            "No commits between these.",
        );
        return;
    }
    let shown = c.commits.len();
    if (shown as u64) < c.total_commits {
        let more = format!(
            "The first {shown} of {} commits (o shows them all on GitHub).",
            c.total_commits
        );
        page.wrapped(vec![Seg::new(more, Role::Meta)], 0, Frame::None);
        page.blank();
    }
    commit_rows(page, repo, &c.commits, None, now);
}
// ---- deployments ------------------------------------------------------------------------------

/// A repository's deployments, newest first, with the environments to
/// filter by.
pub fn deployments(
    page: &mut Page,
    repo: &RepoId,
    list: Option<&DeploymentList>,
    environment: Option<&str>,
    now: u64,
) {
    let Some(l) = list else {
        page.line(vec![Seg::new("Loading deployments…", Role::Meta)]);
        return;
    };
    let base = format!("{}/deployments", url::repo(repo));
    let filter = |env: &str| {
        format!(
            "{base}/activity_log?environments_filter={}",
            url::encode(env)
        )
    };
    if !l.environments.is_empty() {
        let role = |on: bool| if on { Role::Strong } else { Role::Link };
        let mut segs = vec![link_seg(
            page,
            "All",
            base.clone(),
            role(environment.is_none()),
        )];
        for env in &l.environments {
            segs.push(Seg::new(" · ", Role::Meta));
            segs.push(link_seg(
                page,
                env.clone(),
                filter(env),
                role(environment == Some(env.as_str())),
            ));
        }
        page.wrapped(segs, 0, Frame::None);
        page.blank();
    }
    let r = &l.results;
    let title = vec![Seg::new(
        format!("Deployments  {}", compact(r.total)),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    if r.items.is_empty() {
        empty_row(page, "No deployments.");
    }
    box_rows(page, &r.items, |page, d| {
        let target = d
            .log_url
            .clone()
            .unwrap_or_else(|| url::commit(repo, &d.oid));
        item(page, target, |page, link| {
            let (mark, role) = outcome_mark(d.outcome);
            let segs = vec![
                Seg::new(format!("{mark} "), role),
                Seg::linked(d.environment.clone(), Role::Strong, link),
                Seg::new(format!("  {}", d.state), Role::Meta),
            ];
            let right = vec![Seg::new(time::ago_iso(&d.created_at, now), Role::Meta)];
            page.box_line(segs, right, 0);
            let sha = crate::text::short_sha(&d.oid);
            let mut meta = vec![link_seg(page, sha, url::commit(repo, &d.oid), Role::Code)];
            if let Some(branch) = &d.branch {
                meta.push(Seg::new(" · ", Role::Meta));
                meta.push(link_seg(
                    page,
                    branch.clone(),
                    url::tree(repo, branch, ""),
                    Role::Code,
                ));
            }
            if let Some(who) = &d.creator {
                meta.push(Seg::new(" · ", Role::Meta));
                meta.push(link_seg(page, who.clone(), url::user(who), Role::Meta));
            }
            if let Some(at) = &d.environment_url {
                meta.push(Seg::new(" · ", Role::Meta));
                meta.push(link_seg(page, at.clone(), at.clone(), Role::Link));
            }
            body(page, meta);
        });
    });
    more_row(page, r.next.is_some(), r.items.len(), r.total);
    page.box_bottom();
}
// ---- milestones -------------------------------------------------------------------------------

/// How far along a milestone is: a bar, the share done, and the counts.
fn progress(m: &MilestoneInfo) -> Vec<Seg> {
    let all = m.open + m.done;
    let percent = (m.done * 100).checked_div(all).unwrap_or(0);
    let filled = usize::try_from(percent / 10).unwrap_or(0);
    vec![
        Seg::new("▰".repeat(filled), Role::Success),
        Seg::new("▱".repeat(10 - filled), Role::Meta),
        Seg::new(
            format!("  {percent}% · {} open · {} closed", m.open, m.done),
            Role::Meta,
        ),
    ]
}

/// When a milestone is due, or was closed.
fn due(m: &MilestoneInfo) -> String {
    match (&m.closed_at, &m.due_on) {
        (Some(at), _) if m.closed => format!("Closed {}", day(at)),
        (_, Some(at)) => format!("Due by {}", day(at)),
        _ => "No due date".to_owned(),
    }
}

/// A repository's open or closed milestones.
pub fn milestones(
    page: &mut Page,
    repo: &RepoId,
    list: Option<&MilestoneList>,
    closed: bool,
    now: u64,
) {
    let Some(l) = list else {
        page.line(vec![Seg::new("Loading milestones…", Role::Meta)]);
        return;
    };
    let base = format!("{}/milestones", url::repo(repo));
    let role = |on: bool| if on { Role::Strong } else { Role::Link };
    let open = link_seg(
        page,
        format!("Open {}", l.open),
        base.clone(),
        role(!closed),
    );
    let shut = link_seg(
        page,
        format!("Closed {}", l.closed),
        format!("{base}?state=closed"),
        role(closed),
    );
    page.wrapped(
        vec![open, Seg::new(" · ", Role::Meta), shut],
        0,
        Frame::None,
    );
    page.blank();
    let r = &l.results;
    let title = vec![Seg::new(
        format!("Milestones  {}", compact(r.total)),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    if r.items.is_empty() {
        empty_row(page, "No milestones.");
    }
    box_rows(page, &r.items, |page, m| {
        let target = format!("{}/milestone/{}", url::repo(repo), m.number);
        item(page, target, |page, link| {
            let right = vec![Seg::new(due(m), Role::Meta)];
            page.box_line(
                vec![Seg::linked(m.title.clone(), Role::Strong, link)],
                right,
                0,
            );
            let mut segs = progress(m);
            segs.push(Seg::new(
                format!(" · updated {}", time::ago_iso(&m.updated_at, now)),
                Role::Meta,
            ));
            body(page, segs);
            if let Some(first) = m.description.lines().find(|l| !l.trim().is_empty()) {
                body(page, vec![Seg::new(first.trim().to_owned(), Role::Body)]);
            }
        });
    });
    more_row(page, r.next.is_some(), r.items.len(), r.total);
    page.box_bottom();
}

/// A milestone: how far along it is, its description, and its issues and
/// pull requests.
pub fn milestone(page: &mut Page, repo: &RepoId, d: &MilestoneDetail, icons: Icons, now: u64) {
    let m = &d.info;
    page.wrapped(vec![Seg::new(m.title.clone(), Role::Title)], 0, Frame::None);
    let state = if m.closed {
        chip("Closed", Bg::TertiaryContainer)
    } else {
        chip("Open", Bg::SuccessContainer)
    };
    page.wrapped(
        vec![state, Seg::new(format!("  {}", due(m)), Role::Meta)],
        0,
        Frame::None,
    );
    page.wrapped(progress(m), 0, Frame::None);
    if !m.description.trim().is_empty() {
        page.blank();
        let base = LinkBase::new(repo, "HEAD", "");
        markdown::render(page, &m.description, Some(&base), Frame::None);
    }
    page.blank();
    let r = &d.items;
    let title = vec![Seg::new(
        format!("Issues and pull requests  {}", compact(r.total)),
        Role::Strong,
    )];
    page.box_top(title, Vec::new());
    if r.items.is_empty() {
        empty_row(page, "Nothing in this milestone.");
    }
    box_rows(page, &r.items, |page, i| {
        issue_row(page, i, false, icons, now);
    });
    more_row(page, r.next.is_some(), r.items.len(), r.total);
    page.box_bottom();
}
// ---- commits -------------------------------------------------------------------------------

/// A commit: its message, who and when, its parents, and a link to its
/// files (`files` is that link's target).
pub fn commit(page: &mut Page, repo: &RepoId, d: &CommitDetail, files: &str, now: u64) {
    page.wrapped(
        vec![Seg::new(d.headline.clone(), Role::Title)],
        0,
        Frame::None,
    );
    if !d.body.is_empty() {
        page.blank();
        let base = LinkBase::new(repo, &d.oid, "");
        markdown::render(page, &d.body, Some(&base), Frame::None);
    }
    page.blank();
    let mut who = vec![
        link_seg(page, d.author.clone(), url::user(&d.author), Role::Strong),
        Seg::new(
            format!(" authored {}", time::ago_iso(&d.authored_at, now)),
            Role::Meta,
        ),
    ];
    if let Some(committer) = &d.committer {
        who.push(Seg::new(" · ", Role::Meta));
        who.push(link_seg(
            page,
            committer.clone(),
            url::user(committer),
            Role::Strong,
        ));
        who.push(Seg::new(
            format!(" committed {}", time::ago_iso(&d.committed_at, now)),
            Role::Meta,
        ));
    }
    match d.verified {
        Some(true) => who.extend([space(), chip("Verified", Bg::SuccessContainer)]),
        Some(false) => who.extend([space(), chip("Unverified", Bg::ErrorContainer)]),
        None => {}
    }
    page.wrapped(who, 0, Frame::None);
    let mut ids = vec![Seg::new(
        format!("commit {}", crate::text::short_sha(&d.oid)),
        Role::Meta,
    )];
    if !d.parents.is_empty() {
        let noun = if d.parents.len() == 1 {
            "parent"
        } else {
            "parents"
        };
        ids.push(Seg::new(format!(" · {noun} "), Role::Meta));
        for (i, parent) in d.parents.iter().enumerate() {
            if i > 0 {
                ids.push(Seg::new(" + ", Role::Meta));
            }
            let short = crate::text::short_sha(parent);
            ids.push(link_seg(page, short, url::commit(repo, parent), Role::Code));
        }
    }
    page.wrapped(ids, 0, Frame::None);
    page.blank();
    page.box_top(vec![Seg::new("± Files changed", Role::Meta)], Vec::new());
    item(page, files, |page, link| {
        let what = match d.changed_files {
            Some(n) => plural(n, "file"),
            None => "Files".to_owned(),
        };
        let mut segs = vec![Seg::linked(what, Role::Link, link), space()];
        segs.extend(changes(d.additions, d.deletions));
        page.box_line(segs, vec![Seg::new("↵ show the diff", Role::Meta)], 0);
    });
    page.box_bottom();
}

// ---- profile -------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ProfileTab {
    #[default]
    Overview,
    Repositories(RepoSort),
    Stars,
    Followers,
    Following,
    /// An organization's public members.
    People,
}

/// The list a profile tab shows, once loaded.
#[derive(Debug, Clone, Copy)]
pub enum ProfileList<'a> {
    Repos(Option<&'a Results<RepoSummary>>),
    People(Option<&'a Results<UserSummary>>),
    /// The overview's are in the profile.
    None,
}

pub fn profile(page: &mut Page, p: &Profile, tab: ProfileTab, list: ProfileList<'_>, now: u64) {
    let mut segs = Vec::new();
    match &p.name {
        Some(name) => {
            segs.push(Seg::new(name.clone(), Role::Title));
            segs.push(Seg::new(format!("  {}", p.login), Role::Meta));
        }
        None => segs.push(Seg::new(p.login.clone(), Role::Title)),
    }
    if p.is_org {
        segs.push(space());
        segs.push(chip("Organization", Bg::SecondaryContainer));
    }
    if p.verified {
        segs.push(space());
        segs.push(chip("✓ Verified", Bg::SuccessContainer));
    }
    page.line(segs);
    if let Some(status) = &p.status {
        page.wrapped(vec![Seg::new(status.clone(), Role::Meta)], 0, Frame::None);
    }
    if let Some(bio) = &p.bio {
        page.wrapped(vec![Seg::new(bio.clone(), Role::Body)], 0, Frame::None);
    }
    let mut facts = Vec::new();
    if let (Some(followers), Some(following)) = (p.followers, p.following) {
        facts.push(Seg::new(
            format!(
                "⚇ {} followers · {} following",
                compact(followers),
                compact(following)
            ),
            Role::Meta,
        ));
    }
    // Spaced apart, with nothing before the first.
    let gap = |segs: &Vec<Seg>| if segs.is_empty() { "" } else { "   " };
    for (icon, value) in [("◆ ", &p.company), ("⌖ ", &p.location), ("", &p.pronouns)] {
        if let Some(v) = value {
            facts.push(Seg::new(format!("{}{icon}{v}", gap(&facts)), Role::Meta));
        }
    }
    if let Some(site) = &p.website {
        facts.push(Seg::new(format!("{}↗ ", gap(&facts)), Role::Meta));
        facts.push(link_seg(page, site.clone(), site.clone(), Role::Link));
    }
    if !facts.is_empty() {
        page.wrapped(facts, 0, Frame::None);
    }
    let mut links = Vec::new();
    for (shown, target) in &p.socials {
        links.push(Seg::new(format!("{}↗ ", gap(&links)), Role::Meta));
        links.push(link_seg(page, shown.clone(), target.clone(), Role::Link));
    }
    if !links.is_empty() {
        page.wrapped(links, 0, Frame::None);
    }
    if !p.orgs.is_empty() {
        let mut orgs = vec![Seg::new("Organizations  ", Role::Meta)];
        for (i, org) in p.orgs.iter().enumerate() {
            if i > 0 {
                orgs.push(Seg::new(" · ", Role::Meta));
            }
            orgs.push(link_seg(page, org.clone(), url::user(org), Role::Link));
        }
        page.wrapped(orgs, 0, Frame::None);
    }
    if tab == ProfileTab::Overview
        && let Some(readme) = &p.readme
    {
        page.blank();
        let (repo, path) = if p.is_org {
            (format!("{}/.github", p.login), "profile/README.md")
        } else {
            (format!("{0}/{0}", p.login), "README.md")
        };
        let title = Seg::new(format!("{repo} / {path}"), Role::Meta);
        page.box_top(vec![title], Vec::new());
        if let Some(repo) = RepoId::parse(&repo) {
            let base = LinkBase::new(&repo, "HEAD", path);
            markdown::render(page, readme, Some(&base), Frame::Body);
        }
        page.box_bottom();
    }
    page.blank();
    match (tab, list) {
        (ProfileTab::Overview, _) => {}
        (ProfileTab::Repositories(sort), ProfileList::Repos(repos)) => {
            let title = vec![Seg::new(
                format!("Repositories  {}", compact(p.repo_count)),
                Role::Strong,
            )];
            let sort = link_seg(
                page,
                format!("Sort: {}", sort.label()),
                Link::Sort,
                Role::Link,
            );
            repo_list_box(
                page,
                title,
                vec![sort],
                repos,
                false,
                "No public repositories yet.",
                now,
            );
            return;
        }
        (_, ProfileList::Repos(repos)) => {
            let title = vec![Seg::new(
                format!("Starred  {}", compact(p.star_count)),
                Role::Strong,
            )];
            let sort = vec![Seg::new("Recently starred", Role::Meta)];
            repo_list_box(page, title, sort, repos, true, "Nothing starred yet.", now);
            return;
        }
        (tab, ProfileList::People(people)) => {
            let title = match tab {
                ProfileTab::Followers => "Followers",
                ProfileTab::Following => "Following",
                _ => "People",
            };
            people_list(page, title, people);
            return;
        }
        (_, ProfileList::None) => {
            page.line(vec![Seg::new("Loading…", Role::Meta)]);
            return;
        }
    }
    // The overview: pinned (or popular) repositories, then the rest.
    let (title, repos, show_owner) = if p.pinned.is_empty() {
        let mut popular = p.repos.clone();
        popular.sort_by_key(|r| std::cmp::Reverse(r.stars));
        popular.truncate(6);
        ("Popular repositories", popular, false)
    } else {
        ("Pinned", p.pinned.clone(), true)
    };
    let title = vec![Seg::new(title, Role::Strong)];
    list_box(
        page,
        title,
        Vec::new(),
        &repos,
        "No public repositories yet.",
        |page, r| {
            repo_row(page, r, now, show_owner);
        },
    );
    if let Some(c) = &p.contributions {
        page.blank();
        contribution_graph(page, c);
        page.blank();
        contribution_activity(page, c);
    }
    if p.is_org {
        page.blank();
        let title = vec![
            Seg::new("People", Role::Strong),
            Seg::new(format!("  {}", compact(p.people_count)), Role::Meta),
        ];
        page.box_top(title, Vec::new());
        if p.people.is_empty() {
            empty_row(page, "No public members.");
        } else {
            let mut people = Vec::new();
            for (i, login) in p.people.iter().enumerate() {
                if i > 0 {
                    people.push(Seg::new(" · ", Role::Meta));
                }
                people.push(link_seg(page, login.clone(), url::user(login), Role::Link));
            }
            page.wrapped(people, 0, Frame::Body);
        }
        page.box_bottom();
        let languages = top_languages(&p.repos);
        if !languages.is_empty() {
            page.blank();
            let mut segs = vec![Seg::new("Top languages  ", Role::Strong)];
            for (lang, n) in languages {
                segs.push(Seg::new("● ", Role::Accent));
                segs.push(Seg::new(format!("{lang} {n}   "), Role::Meta));
            }
            page.wrapped(segs, 0, Frame::None);
        }
    }
}

/// 1234567 → "1,234,567".
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// "Sep" for `2026-09-…`.
fn month_name(date: &str) -> Option<&'static str> {
    let month: usize = date.get(5..7)?.parse().ok()?;
    MONTHS.get(month.checked_sub(1)?).copied()
}

/// The last year's contributions as GitHub draws them: a column per week,
/// a row per day, shaded by how much happened. Narrow pages show the most
/// recent weeks that fit.
fn contribution_graph(page: &mut Page, c: &Contributions) {
    const LABELS: usize = 4;
    let title = format!("{} contributions in the last year", thousands(c.total));
    page.line(vec![Seg::new(title, Role::Strong)]);
    let room = usize::from(page.room(Frame::None, 0)).saturating_sub(LABELS);
    let weeks = c
        .weeks
        .get(c.weeks.len().saturating_sub(room)..)
        .unwrap_or_default();
    // Month names over the week each month starts in, where they fit.
    let mut row = vec![' '; weeks.len()];
    let (mut free, mut last) = (0, None);
    for (i, week) in weeks.iter().enumerate() {
        let month = month_name(&week.start);
        if month != last
            && i >= free
            && let Some(name) = month
            && i + name.len() <= row.len()
        {
            for (cell, ch) in row.iter_mut().skip(i).zip(name.chars()) {
                *cell = ch;
            }
            free = i + name.len() + 1;
        }
        last = month;
    }
    let months: String = " ".repeat(LABELS).chars().chain(row).collect();
    page.line(vec![Seg::new(months, Role::Meta)]);
    for day in 0..7 {
        let label = match day {
            1 => "Mon ",
            3 => "Wed ",
            5 => "Fri ",
            _ => "    ",
        };
        let mut segs = vec![Seg::new(label, Role::Meta)];
        for week in weeks {
            segs.push(match week.days.get(day) {
                Some(&level) => Seg::new("■", Role::Heat(level.min(4))),
                None => Seg::new(" ", Role::Meta),
            });
        }
        page.line(segs);
    }
    let mut legend = vec![Seg::new(format!("{}Less ", " ".repeat(LABELS)), Role::Meta)];
    legend.extend((0..=4).map(|level| Seg::new("■", Role::Heat(level))));
    legend.push(Seg::new(" More", Role::Meta));
    page.line(legend);
}

/// What someone did, month by month: commits by repository, and the pull
/// requests and issues they opened and reviewed.
fn contribution_activity(page: &mut Page, c: &Contributions) {
    page.line(vec![Seg::new("Contribution activity", Role::Strong)]);
    if c.activity.is_empty() {
        page.line(vec![Seg::new("No recent activity.", Role::Meta)]);
        return;
    }
    for m in &c.activity {
        page.blank();
        let year = m.month.get(..4).unwrap_or_default();
        let name = month_name(&format!("{}-01", m.month)).unwrap_or_default();
        page.line(vec![Seg::new(format!("{name} {year}"), Role::Heading)]);
        if !m.commits.is_empty() {
            let total: u64 = m.commits.iter().map(|(_, n)| n).sum();
            let repos = m.commits.len() as u64;
            let repos = if repos == 1 {
                "1 repository".to_owned()
            } else {
                format!("{repos} repositories")
            };
            let created = format!("◷ Created {} in {repos}", plural(total, "commit"));
            page.line(vec![Seg::new(created, Role::Body)]);
            for (repo, n) in m.commits.iter().take(5) {
                let name = link_seg(
                    page,
                    repo.clone(),
                    format!("{}/{repo}", url::BASE),
                    Role::Link,
                );
                page.line(vec![
                    Seg::new("    ", Role::Meta),
                    name,
                    Seg::new(format!("  {}", plural(*n, "commit")), Role::Meta),
                ]);
            }
        }
        for (items, icon, verb) in [
            (&m.pulls, "⇄", "Opened"),
            (&m.issues, "◉", "Opened"),
            (&m.reviews, "◎", "Reviewed"),
        ] {
            if items.is_empty() {
                continue;
            }
            let noun = if icon == "◉" {
                "issue"
            } else {
                "pull request"
            };
            let what = plural(items.len() as u64, noun);
            page.line(vec![Seg::new(format!("{icon} {verb} {what}"), Role::Body)]);
            for item in items.iter().take(5) {
                let target = if icon == "◉" {
                    format!("{}/{}/issues/{}", url::BASE, item.repo, item.number)
                } else {
                    format!("{}/{}/pull/{}", url::BASE, item.repo, item.number)
                };
                let place = format!("{}#{}", item.repo, item.number);
                let link = link_seg(page, item.title.clone(), target, Role::Link);
                page.wrapped(
                    vec![Seg::new(format!("{place}  "), Role::Meta), link],
                    4,
                    Frame::None,
                );
            }
        }
    }
}

/// An organization's most used languages, across the repositories loaded.
fn top_languages(repos: &[RepoSummary]) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for lang in repos.iter().filter_map(|r| r.language.as_ref()) {
        match counts.iter_mut().find(|(l, _)| l == lang) {
            Some((_, n)) => *n += 1,
            None => counts.push((lang.clone(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts.truncate(5);
    counts
}

// ---- home -----------------------------------------------------------------------------------

pub fn home(
    page: &mut Page,
    inbox: Option<&Inbox>,
    repos: Option<&[RepoSummary]>,
    viewer: Option<&str>,
    icons: Icons,
    now: u64,
) {
    let pr_box =
        |page: &mut Page, title: &str, total: u64, prs: &[PrSummary], query: &str, empty: &str| {
            let all = link_seg(
                page,
                "View all",
                url::search(SearchKind::Pulls, query),
                Role::Link,
            );
            let total = compact(total.max(prs.len() as u64));
            let title = vec![
                Seg::new(title.to_owned(), Role::Strong),
                Seg::new(format!("  {total}"), Role::Meta),
            ];
            list_box(page, title, vec![all], prs, empty, |page, p| {
                let summary = IssueSummary {
                    repo: p.pr.repo.clone(),
                    number: p.pr.number,
                    title: p.title.clone(),
                    is_pr: true,
                    state: pr_state(p.state),
                    author: p.author.clone(),
                    updated_at: p.updated_at.clone(),
                    comments: p.comments,
                    labels: Vec::new(),
                    created_at: String::new(),
                    review: p.review,
                    checks: p.checks,
                };
                issue_row(page, &summary, true, icons, now);
            });
        };
    match inbox {
        Some(inbox) => {
            pr_box(
                page,
                "Review requests",
                inbox.review_requested_total,
                &inbox.review_requested,
                "is:open is:pr review-requested:@me archived:false",
                "Nothing is waiting for your review.",
            );
            pr_box(
                page,
                "Your pull requests",
                inbox.authored_total,
                &inbox.authored,
                "is:open is:pr author:@me archived:false",
                "You have no open pull requests.",
            );
        }
        None => loading_box(page, vec![Seg::new("Review requests", Role::Strong)]),
    }
    let all = viewer.map(|login| {
        link_seg(
            page,
            "View all",
            format!("{}?tab=repositories", url::user(login)),
            Role::Link,
        )
    });
    let title = vec![Seg::new("Your repositories", Role::Strong)];
    match repos {
        Some(repos) => {
            let empty = "You don't have any repositories yet.";
            let all = Vec::from_iter(all);
            list_box(page, title, all, repos, empty, |page, r| {
                repo_row(page, r, now, true);
            });
        }
        None => loading_box(page, title),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Steps that start in the second the one before ends still get their
    /// own lines, by the lines that start them.
    #[test]
    fn log_lines_go_to_their_steps() {
        let step = |number, outcome, start: &str, end: &str| ghtui_api::browse::Step {
            number,
            name: String::new(),
            outcome,
            started_at: Some(format!("2026-10-05T17:42:{start}Z")),
            completed_at: Some(format!("2026-10-05T17:42:{end}Z")),
        };
        let job = Job {
            id: 1,
            run_id: 1,
            name: String::new(),
            outcome: CheckOutcome::Success,
            started_at: None,
            completed_at: None,
            steps: vec![
                step(1, CheckOutcome::Success, "04", "04"),
                step(2, CheckOutcome::Success, "04", "16"),
                step(3, CheckOutcome::Skipped, "16", "16"),
                step(4, CheckOutcome::Success, "16", "37"),
                step(8, CheckOutcome::Success, "37", "37"),
                step(9, CheckOutcome::Success, "37", "37"),
            ],
        };
        let log = [
            "\u{feff}2026-10-05T17:42:04.31Z Current runner version",
            "2026-10-05T17:42:04.32Z ##[group]Runner Image",
            "2026-10-05T17:42:04.90Z ##[group]Run actions/checkout@v4",
            "2026-10-05T17:42:16.01Z HEAD is now at 984a54e",
            "2026-10-05T17:42:16.07Z ##[group]Run cd src/ci/citool",
            "2026-10-05T17:42:37.10Z done",
            "2026-10-05T17:42:37.20Z Post job cleanup.",
            "2026-10-05T17:42:37.49Z Cleaning up orphan processes",
        ]
        .join("\n");
        let lines = step_lines(&job, &log);
        let of = |n: u32| {
            lines
                .iter()
                .find(|(s, _)| *s == n)
                .map(|(_, l)| l.clone())
                .unwrap_or_default()
        };
        assert_eq!(of(1), ["Current runner version", "##[group]Runner Image"]);
        assert_eq!(
            of(2),
            ["##[group]Run actions/checkout@v4", "HEAD is now at 984a54e"]
        );
        assert!(of(3).is_empty());
        assert_eq!(of(4), ["##[group]Run cd src/ci/citool", "done"]);
        assert_eq!(of(8), ["Post job cleanup."]);
        assert_eq!(of(9), ["Cleaning up orphan processes"]);
    }

    #[test]
    fn escapes_are_stripped_from_logs() {
        assert_eq!(strip_escapes("\u{1b}[36;1mcd src\u{1b}[0m"), "cd src");
        assert_eq!(strip_escapes("a\u{1b}]8;;https://x\u{7}b"), "ab");
        assert_eq!(strip_escapes("plain [36m"), "plain [36m");
    }

    #[test]
    fn wiki_links_become_markdown() {
        assert_eq!(wiki_links("See [[FAQ]]."), "See [FAQ](FAQ).");
        assert_eq!(
            wiki_links("[[Start here|Getting started]]"),
            "[Start here](Getting-started)"
        );
        assert_eq!(wiki_links("[[unclosed"), "[[unclosed");
    }

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

    #[test]
    fn repo_code_page_has_files_and_readme_links() {
        let repo = RepoId::new("o", "r");
        let overview = RepoOverview {
            summary: RepoSummary {
                repo: repo.clone(),
                description: Some("A thing".into()),
                stars: 1200,
                forks: 30,
                language: Some("Rust".into()),
                language_color: None,
                pushed_at: None,
                private: false,
                fork: false,
                archived: false,
            },
            homepage: None,
            watchers: 5,
            open_issues: 3,
            open_prs: 1,
            closed_issues: 9,
            closed_prs: 4,
            license: Some("MIT".into()),
            topics: vec!["tui".into()],
            default_branch: Some("main".into()),
            last_commit: None,
            commits: 10,
            parent: None,
            starred: false,
            id: ghtui_api::model::NodeId::new("R_1"),
            has_issues: true,
            entries: vec![
                TreeEntry {
                    name: "src".into(),
                    path: "src".into(),
                    kind: EntryKind::Dir,
                    size: None,
                },
                TreeEntry {
                    name: "README.md".into(),
                    path: "README.md".into(),
                    kind: EntryKind::File,
                    size: Some(120),
                },
            ],
            readme: Some(ghtui_api::browse::Readme {
                path: "README.md".into(),
                text: "# Hello\n\nSee [docs](docs/intro.md).".into(),
            }),
        };
        let mut page = Page::new(80);
        repo_title(&mut page, &repo, Some(&overview));
        repo_code(&mut page, &repo, &overview, None, None, PageCtx::default());
        for link in [
            Link::from("https://github.com/o/r/tree/main/src"),
            Link::from("https://github.com/o/r/blob/main/README.md"),
            Link::from("https://github.com/o/r/blob/main/docs/intro.md"),
            Link::from("https://github.com/o"),
            Link::Star,
            Link::FindFile,
        ] {
            assert!(
                page.links.contains(&link),
                "{link:?} missing: {:?}",
                page.links
            );
        }
        let text: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert!(text.iter().any(|l| l.contains("1.2k stars")), "{text:?}");
        assert!(text.iter().any(|l| l == "Hello"), "{text:?}");
        // Files and the `..`-less root list are items, in order.
        let opened: Vec<&str> = page
            .items
            .iter()
            .filter_map(|i| page.links[i.link as usize].url())
            .collect();
        assert_eq!(
            opened,
            [
                "https://github.com/o/r/tree/main/src",
                "https://github.com/o/r/blob/main/README.md"
            ]
        );
    }

    #[test]
    fn wide_pages_get_an_about_sidebar() {
        assert_eq!(aside_width(90), None);
        assert_eq!(main_width(120, aside_width(120)), 120 - 30 - ASIDE_GAP);
    }

    #[test]
    fn list_queries_name_their_state_and_sort() {
        assert_eq!(list_state("is:open label:bug"), "open");
        assert_eq!(list_state("is:closed"), "closed");
        assert_eq!(list_state("label:bug"), "all");
        assert_eq!(sort_label("is:open sort:comments-desc"), "Most commented");
        assert_eq!(sort_label("is:open"), "Newest");
        assert_eq!(day("2026-10-01T12:00:00Z"), "Oct 1, 2026");
    }
}
