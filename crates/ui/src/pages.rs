//! Page builders: GitHub's pages (repository, file, lists, issue, pull
//! request, profile, search, home) as [`Page`]s, laid out the way GitHub
//! lays them out: boxes for file lists, READMEs, lists and comments, and a
//! sidebar when there's room. Every link is the github.com URL of what it
//! points at; `ghtui:` links are actions (star, load more, filter...).

use std::collections::HashMap;

use ghtui_api::browse::{
    Blob, Comment, CommitInfo, EntryKind, IssueDetail, IssueState, IssueSummary, PrActivity,
    Profile, RepoOverview, RepoSummary, Results, SearchKind, SearchResults, TreeEntry, UserSummary,
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
        };
        format!("{BASE}/search?q={}&type={kind}", encode(query))
    }
    pub fn commit(repo: &RepoId, oid: &str) -> String {
        format!("{BASE}/{repo}/commit/{oid}")
    }

    /// A file path in a URL: `/` separates segments, everything else that
    /// isn't plainly safe (`#`, `?`, `%`, `\\`, spaces, non-ASCII) is
    /// escaped.
    pub fn encode_path(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                    out.push(char::from(b));
                }
                b => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    /// Minimal query-string encoding.
    pub fn encode(s: &str) -> String {
        let mut out = String::new();
        for b in s.bytes() {
            match b {
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'-'
                | b'_'
                | b'.'
                | b'~'
                | b':'
                | b'/'
                | b'@' => out.push(b as char),
                b' ' => out.push('+'),
                b => out.push_str(&format!("%{b:02X}")),
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
        page.box_line(
            vec![Seg::new(" ".repeat(width), Role::Chip(Bg::ContainerHigh))],
            Vec::new(),
            0,
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

/// A box's empty state.
fn empty_row(page: &mut Page, text: &str) {
    page.box_line(vec![Seg::new(text, Role::Meta)], Vec::new(), 0);
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
        right.push(space());
        right.push(button(
            page,
            format!("⑂ Fork {}", compact(s.forks)),
            format!("{}/forks", url::repo(repo)),
        ));
        right.push(space());
        right.push(button(
            page,
            format!("◉ Watch {}", compact(o.watchers)),
            format!("{}/watchers", url::repo(repo)),
        ));
    }
    page.push(PageLine {
        segs,
        right,
        ..PageLine::default()
    });
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
                format!("{}/commits/{rev}", url::repo(repo)),
                Role::Meta,
            ),
        );
    }
    page.push(PageLine {
        segs,
        right,
        ..PageLine::default()
    });
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
        let start = page.lines.len();
        let link = page.link(url::tree(repo, rev, parent));
        page.box_line(vec![Seg::linked("..", Role::Link, link)], Vec::new(), 2);
        page.item(start, link);
    }
    match entries {
        Some([]) => empty_row(page, "This directory is empty."),
        None => skeleton(page),
        Some(entries) => {
            for e in entries {
                let (icon, target, role) = match e.kind {
                    EntryKind::Dir => (icons.folder(), url::tree(repo, rev, &e.path), Role::Link),
                    EntryKind::Submodule => ("⊙", url::tree(repo, rev, &e.path), Role::Link),
                    EntryKind::Symlink => ("↪", url::blob(repo, rev, &e.path), Role::Body),
                    EntryKind::File => (icons.file(), url::blob(repo, rev, &e.path), Role::Body),
                };
                let start = page.lines.len();
                let link = page.link(target);
                let icon_role = if e.kind == EntryKind::Dir {
                    Role::Accent
                } else {
                    Role::Meta
                };
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
                page.item(start, link);
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
        page.box_top(
            vec![Seg::new("This repository is empty.", Role::Strong)],
            Vec::new(),
        );
        empty_row(page, "Nothing has been pushed yet.");
        page.box_bottom();
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
pub fn file(page: &mut Page, repo: &RepoId, rev: &str, path: &str, blob: &Blob, keys: Keys<'_>) {
    code_toolbar(page, repo, rev, path, true, 0, keys);
    let size = crate::text::size(blob.size);
    let Some(text) = &blob.text else {
        page.box_top(vec![Seg::new(size, Role::Meta)], Vec::new());
        empty_row(page, "Binary file not shown. o opens it on GitHub.");
        page.box_bottom();
        return;
    };
    let lines = text.lines().count() as u64;
    let mut info = format!("{} · {size}", plural(lines, "line"));
    if blob.truncated {
        info.push_str(" · only the beginning is shown");
    }
    let raw = link_seg(
        page,
        "Raw",
        format!("https://raw.githubusercontent.com/{repo}/{rev}/{path}"),
        Role::Link,
    );
    page.box_top(vec![Seg::new(info, Role::Meta)], vec![raw]);
    if path.to_ascii_lowercase().ends_with(".md") {
        let base = LinkBase::new(repo, rev, path);
        markdown::render(page, text, Some(&base), Frame::Body);
        page.box_bottom();
        return;
    }
    let lines = markdown::highlighted(text, ghtui_diff::Language::from_path(path));
    let width = lines.len().to_string().len();
    for (i, mut segs) in lines.into_iter().enumerate() {
        let number = format!("{:>width$}  ", i + 1);
        segs.insert(0, Seg::new(number, Role::Syntax(Syntax::Comment)));
        page.push(PageLine {
            segs,
            frame: Frame::Body,
            tone: Tone::Code,
            ..PageLine::default()
        });
    }
    page.box_bottom();
}

// ---- lists --------------------------------------------------------------------------

fn repo_row(page: &mut Page, r: &RepoSummary, now: u64, show_owner: bool) {
    let start = page.lines.len();
    let name = if show_owner {
        r.repo.to_string()
    } else {
        r.repo.name.clone()
    };
    let link = page.link(url::repo(&r.repo));
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
        page.item(start, link);
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
        meta.push(Seg::new(
            format!("   Updated {}", time::ago_iso(p, now)),
            Role::Meta,
        ));
    }
    page.box_line(meta, Vec::new(), 0);
    page.item(start, link);
}

/// A box of repositories.
fn repo_box(
    page: &mut Page,
    title: Vec<Seg>,
    right: Vec<Seg>,
    repos: &[RepoSummary],
    empty: &str,
    show_owner: bool,
    now: u64,
) {
    page.box_top(title, right);
    if repos.is_empty() {
        empty_row(page, empty);
    }
    box_rows(page, repos, |page, r| repo_row(page, r, now, show_owner));
    page.box_bottom();
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
    let start = page.lines.len();
    let link = page.link(target);
    let place = if show_repo {
        format!("{}#{}", i.repo, i.number)
    } else {
        format!("#{}", i.number)
    };
    if page.compact {
        // One row: the title, then where it is and its signals.
        let segs = vec![
            Seg::new(format!("{icon} "), role),
            Seg::linked(i.title.clone(), Role::Strong, link),
        ];
        let mut right = vec![Seg::new(format!("{place}  "), Role::Meta)];
        right.extend(issue_signals(i, icons));
        page.box_line(segs, right, 0);
        page.item(start, link);
        return;
    }
    let mut segs = vec![Seg::linked(i.title.clone(), Role::Strong, link)];
    labels(&mut segs, &i.labels);
    hanging(page, Seg::new(format!("{icon} "), role), segs, Frame::Body);
    // GitHub's second line: `#12 opened 3 days ago by octocat · Approved`.
    let when = if i.created_at.is_empty() {
        format!(
            "updated {} by {}",
            time::ago_iso(&i.updated_at, now),
            i.author
        )
    } else {
        format!(
            "opened {} by {}",
            time::ago_iso(&i.created_at, now),
            i.author
        )
    };
    let mut meta = vec![Seg::new(format!("{place} {when}"), Role::Meta)];
    if let Some(review) = i.review {
        let (text, role) = match review {
            ReviewDecision::Approved => ("Approved", Role::Success),
            ReviewDecision::ChangesRequested => ("Changes requested", Role::Error),
            ReviewDecision::ReviewRequired => ("Review required", Role::Meta),
        };
        meta.extend([Seg::new(" · ", Role::Meta), Seg::new(text, role)]);
    }
    page.box_line(meta, issue_signals(i, icons), 2);
    page.item(start, link);
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
    let start = page.lines.len();
    let link = page.link(url::user(&u.login));
    let mut segs = vec![Seg::linked(u.login.clone(), Role::Link, link)];
    if let Some(name) = &u.name {
        segs.push(Seg::new(format!("  {name}"), Role::Strong));
    }
    if u.is_org {
        segs.push(space());
        segs.push(chip("Organization", Bg::SecondaryContainer));
    }
    page.box_line(segs, Vec::new(), 0);
    if let Some(bio) = u.bio.as_ref().filter(|_| !page.compact) {
        page.wrapped(vec![Seg::new(bio.clone(), Role::Meta)], 0, Frame::Body);
    }
    page.item(start, link);
}

fn more_row(page: &mut Page, next: bool, shown: usize, total: u64) {
    if next {
        page.box_rule();
        let start = page.lines.len();
        let link = page.link(Link::More);
        page.box_line(
            vec![Seg::linked(
                format!("Load more  ({shown} of {})", compact(total)),
                Role::Link,
                link,
            )],
            Vec::new(),
            0,
        );
        page.item(start, link);
    }
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
        let link = page.link(Link::State(name.to_string()));
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
                issue_row(page, i, false, icons, now)
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
    };
    let Some(results) = results else {
        loading_box(page, vec![Seg::new("Searching…", Role::Meta)]);
        return;
    };
    let (total, shown, next) = match results {
        SearchResults::Repos(r) => (r.total, r.items.len(), r.next.is_some()),
        SearchResults::Issues(r) => (r.total, r.items.len(), r.next.is_some()),
        SearchResults::Users(r) => (r.total, r.items.len(), r.next.is_some()),
    };
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
            issue_row(page, i, true, icons, now)
        }),
        SearchResults::Users(r) => box_rows(page, &r.items, user_row),
    }
    more_row(page, next, shown, total);
    page.box_bottom();
}

// ---- conversations --------------------------------------------------------------------

/// Who said what, when, for a comment box.
#[derive(Clone, Copy)]
struct Said<'a> {
    author: &'a str,
    /// "commented", "opened"...
    verb: &'a str,
    when: &'a str,
    body: &'a str,
    badge: Option<&'a str>,
}

/// A comment box: `╭─ author commented 3d ago ─── Author ─╮`, the Markdown
/// body, `╰──╯`.
fn comment_box(page: &mut Page, said: Said<'_>, base: &LinkBase, now: u64) {
    let Said {
        author,
        verb,
        when,
        body,
        badge,
    } = said;
    let start = page.lines.len();
    let who = link_seg(page, author.to_owned(), url::user(author), Role::Strong);
    let right = badge
        .map(|b| chip(b, Bg::SecondaryContainer))
        .into_iter()
        .collect();
    page.box_top(
        vec![
            who,
            Seg::new(format!(" {verb} {}", time::ago_iso(when, now)), Role::Meta),
        ],
        right,
    );
    if body.trim().is_empty() {
        empty_row(page, "No description provided.");
    } else {
        markdown::render(page, body, Some(base), Frame::Body);
    }
    page.box_bottom();
    // The whole comment is a row: Enter quote-replies, as GitHub's `r` does.
    let quote = page.quote(author, body);
    page.item(start, quote);
}

/// A comment in a conversation, badged when it's by `op`, who opened it.
fn comment(page: &mut Page, c: &Comment, op: &str, base: &LinkBase, now: u64) {
    let badge = (c.author == op).then_some("Author");
    comment_box(
        page,
        Said {
            author: &c.author,
            verb: "commented",
            when: &c.created_at,
            body: &c.body,
            badge,
        },
        base,
        now,
    );
}

/// The line between timeline entries.
fn connector(page: &mut Page) {
    page.push(PageLine {
        segs: vec![Seg::new("│", Role::Meta)],
        indent: 3,
        ..PageLine::default()
    });
}

/// A timeline event: `● author approved these changes 1d ago`.
fn event(
    page: &mut Page,
    icon: &str,
    icon_role: Role,
    author: &str,
    what: &str,
    when: &str,
    now: u64,
) {
    let who = link_seg(page, author.to_owned(), url::user(author), Role::Strong);
    page.push(PageLine {
        segs: vec![
            Seg::new(format!("{icon}  "), icon_role),
            who,
            Seg::new(format!(" {what} {}", time::ago_iso(when, now)), Role::Meta),
        ],
        indent: 2,
        ..PageLine::default()
    });
}

/// The comment box at the end of a conversation.
fn add_comment(page: &mut Page, keys: Keys<'_>) {
    connector(page);
    let start = page.lines.len();
    let link = page.link(Link::Comment);
    page.box_top(
        vec![Seg::linked("Add a comment", Role::Strong, link)],
        Vec::new(),
    );
    page.box_line(
        vec![Seg::linked(
            format!(
                "Press {} to write a comment (Markdown; ctrl-e for $EDITOR)",
                keys.comment
            ),
            Role::Meta,
            link,
        )],
        Vec::new(),
        0,
    );
    page.box_bottom();
    page.item(start, link);
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
    let base = LinkBase::new(&d.repo, "HEAD", "");
    comment_box(
        page,
        Said {
            author: &d.author,
            verb: "opened",
            when: &d.created_at,
            body: &d.body,
            badge: Some("Author"),
        },
        &base,
        now,
    );
    for c in &d.comments {
        connector(page);
        comment(page, c, &d.author, &base, now);
    }
    add_comment(page, keys);
    if let Some(width) = aside {
        let mut participants: Vec<String> = Vec::new();
        for who in std::iter::once(&d.author).chain(d.comments.iter().map(|c| &c.author)) {
            if !participants.contains(who) {
                participants.push(who.clone());
            }
        }
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
    let row = |page: &mut Page, ok: Option<bool>, text: &str| {
        let (icon, role) = match ok {
            Some(true) => ("✓", Role::Success),
            Some(false) => ("✗", Role::Error),
            None => ("●", Role::Accent),
        };
        page.box_line(
            vec![
                Seg::new(format!("{icon}  "), role),
                Seg::new(text, Role::Body),
            ],
            Vec::new(),
            0,
        );
    };
    match s.review {
        Some(ReviewDecision::Approved) => row(page, Some(true), "Changes approved"),
        Some(ReviewDecision::ChangesRequested) => row(page, Some(false), "Changes requested"),
        Some(ReviewDecision::ReviewRequired) => row(page, None, "Review required"),
        None => {}
    }
    match s.checks {
        Some(ChecksState::Passing) => row(page, Some(true), "All checks have passed"),
        Some(ChecksState::Failing) => row(page, Some(false), "Some checks were not successful"),
        Some(ChecksState::Pending) => row(page, None, "Some checks haven't completed yet"),
        None => {}
    }
    match d.mergeable {
        Mergeable::Yes => row(page, Some(true), "No conflicts with the base branch"),
        Mergeable::Conflicting => row(page, Some(false), "This branch has conflicts"),
        Mergeable::Unknown => row(page, None, "Checking for conflicts…"),
    }
    let start = page.lines.len();
    let link = page.link(url::pull_tab(pr, "files"));
    page.box_rule();
    page.box_line(
        vec![
            Seg::new(format!("{}  ", icons.external()), Role::Meta),
            Seg::linked(
                format!("Review the {} changed files", d.changed_files),
                Role::Link,
                link,
            ),
        ],
        Vec::new(),
        0,
    );
    page.item(start + 1, link);
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
    let PageCtx { icons, keys, now } = cx;
    pr_summary(page, pr, d, now);
    let base = LinkBase::new(&pr.repo, &d.head_oid, "");
    comment_box(
        page,
        Said {
            author: &d.summary.author,
            verb: "opened",
            when: &d.created_at,
            body: &d.body,
            badge: Some("Author"),
        },
        &base,
        now,
    );
    let Some(a) = activity else {
        connector(page);
        page.line(vec![Seg::new("   Loading the conversation…", Role::Meta)]);
        return;
    };
    // Comments and reviews in time order.
    enum Entry<'a> {
        Comment(&'a Comment),
        Review(&'a ghtui_api::browse::ReviewSummary),
    }
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
            Entry::Comment(c) => comment(page, c, &d.summary.author, &base, now),
            Entry::Review(r) => {
                let (icon, role, what) = match r.state.as_str() {
                    "approved" => ("✓", Role::Success, "approved these changes"),
                    "requested changes" => ("✗", Role::Error, "requested changes"),
                    "dismissed" => ("○", Role::Meta, "had a review dismissed"),
                    _ => ("◉", Role::Meta, "reviewed"),
                };
                event(page, icon, role, &r.author, what, &r.submitted_at, now);
                if !r.body.trim().is_empty() {
                    connector(page);
                    comment_box(
                        page,
                        Said {
                            author: &r.author,
                            verb: "commented",
                            when: &r.submitted_at,
                            body: &r.body,
                            badge: None,
                        },
                        &base,
                        now,
                    );
                }
            }
        }
    }
    merge_box(page, pr, d, icons);
    add_comment(page, keys);
    if let Some(width) = aside {
        let mut reviewers: Vec<String> = Vec::new();
        for r in &a.reviews {
            if !reviewers.contains(&r.author) {
                reviewers.push(r.author.clone());
            }
        }
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
    let mut current_day = String::new();
    for c in &a.commits {
        let date = day(&c.date);
        if date != current_day {
            if !current_day.is_empty() {
                page.box_bottom();
            }
            page.box_top(
                vec![Seg::new(format!("◷ Commits on {date}"), Role::Meta)],
                Vec::new(),
            );
            current_day = date;
        } else {
            page.box_rule();
        }
        let start = page.lines.len();
        let link = page.link(url::commit(&pr.repo, &c.oid));
        page.box_line(
            vec![Seg::linked(c.headline.clone(), Role::Strong, link)],
            vec![Seg::new(crate::text::short_sha(&c.oid), Role::Code)],
            0,
        );
        page.box_line(
            vec![Seg::new(
                format!("{} committed {}", c.author, time::ago_iso(&c.date, now)),
                Role::Meta,
            )],
            Vec::new(),
            0,
        );
        page.item(start, link);
    }
    if !current_day.is_empty() {
        page.box_bottom();
    }
}

// ---- profile -------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ProfileTab {
    #[default]
    Overview,
    Repositories,
    Stars,
}

pub fn profile(page: &mut Page, p: &Profile, tab: ProfileTab, now: u64) {
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
    page.line(segs);
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
    for (icon, value) in [("◆ ", &p.company), ("⌖ ", &p.location)] {
        if let Some(v) = value {
            facts.push(Seg::new(format!("   {icon}{v}"), Role::Meta));
        }
    }
    if let Some(site) = &p.website {
        facts.push(Seg::new("   ↗ ", Role::Meta));
        facts.push(link_seg(page, site.clone(), site.clone(), Role::Link));
    }
    if !facts.is_empty() {
        page.wrapped(facts, 0, Frame::None);
    }
    let mut popular = Vec::new();
    let (title, sort, repos, empty, show_owner) = match tab {
        ProfileTab::Overview if p.pinned.is_empty() => {
            popular.clone_from(&p.repos);
            popular.sort_by_key(|r| std::cmp::Reverse(r.stars));
            popular.truncate(6);
            let title = "Popular repositories".to_owned();
            (title, None, &popular, "No public repositories yet.", false)
        }
        ProfileTab::Overview => ("Pinned".to_owned(), None, &p.pinned, "", true),
        ProfileTab::Repositories => (
            format!("Repositories  {}", compact(p.repo_count)),
            Some("Sort: Last updated"),
            &p.repos,
            "No public repositories yet.",
            false,
        ),
        ProfileTab::Stars => (
            format!("Starred  {}", compact(p.star_count)),
            Some("Sort: Recently starred"),
            &p.stars,
            "Nothing starred yet.",
            true,
        ),
    };
    let title = vec![Seg::new(title, Role::Strong)];
    let right = sort.map(|s| Seg::new(s, Role::Meta)).into_iter().collect();
    repo_box(page, title, right, repos, empty, show_owner, now);
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
            page.box_top(
                vec![
                    Seg::new(title.to_owned(), Role::Strong),
                    Seg::new(format!("  {}", compact(total)), Role::Meta),
                ],
                vec![all],
            );
            if prs.is_empty() {
                empty_row(page, empty);
            }
            box_rows(page, prs, |page, p| {
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
            page.box_bottom();
        };
    match inbox {
        Some(inbox) => {
            pr_box(
                page,
                "Review requests",
                inbox
                    .review_requested_total
                    .max(inbox.review_requested.len() as u64),
                &inbox.review_requested,
                "is:open is:pr review-requested:@me archived:false",
                "Nothing is waiting for your review.",
            );
            pr_box(
                page,
                "Your pull requests",
                inbox.authored_total.max(inbox.authored.len() as u64),
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
        Some(repos) => repo_box(
            page,
            title,
            all.into_iter().collect(),
            repos,
            "You don't have any repositories yet.",
            true,
            now,
        ),
        None => loading_box(page, title),
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
