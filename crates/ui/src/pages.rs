//! Page builders: GitHub's pages (repository, file, issue, profile, search,
//! lists, home) as [`Page`]s. Every link is the github.com URL of what it
//! points at.

use ghtui_api::browse::{
    EntryKind, IssueDetail, IssueState, IssueSummary, PrActivity, Profile, RepoOverview,
    RepoSummary, Results, SearchKind, SearchResults, TreeEntry, UserSummary,
};
use ghtui_api::model::{
    ChecksState, Inbox, Label, PrDetail, PrRef, PrState, PrSummary, RepoId, ReviewDecision,
};
use ghtui_theme::Bg;

use crate::markdown::{self, LinkBase};
use crate::page::{Page, PageLine, Role, Seg, Surface};
use crate::{Icons, time};

pub const MORE: &str = "ghtui:more";

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
        if path.is_empty() {
            format!("{BASE}/{repo}/tree/{rev}")
        } else {
            format!("{BASE}/{repo}/tree/{rev}/{path}")
        }
    }
    pub fn blob(repo: &RepoId, rev: &str, path: &str) -> String {
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
            SearchKind::Users => "users",
        };
        format!("{BASE}/search?q={}&type={kind}", encode(query))
    }
    pub fn commit(repo: &RepoId, oid: &str) -> String {
        format!("{BASE}/{repo}/commit/{oid}")
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
        1_000..999_950 => trim_zero(format!("{:.1}k", n as f64 / 1_000.0)),
        _ => trim_zero(format!("{:.1}m", n as f64 / 1_000_000.0)),
    }
}

fn trim_zero(s: String) -> String {
    s.replace(".0k", "k").replace(".0m", "m")
}

fn size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

fn plural(n: u64, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
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

fn link_seg(page: &mut Page, text: impl Into<String>, url: impl Into<String>, role: Role) -> Seg {
    let link = page.link(url);
    Seg {
        text: text.into(),
        role,
        link: Some(link),
    }
}

/// A row of tabs; `active` is filled. Each tab is a link.
fn tabs(page: &mut Page, items: &[(String, String, Option<u64>)], active: usize) {
    let mut segs = Vec::new();
    for (i, (label, url, count)) in items.iter().enumerate() {
        if i > 0 {
            segs.push(Seg::new("   ", Role::Body));
        }
        let text = format!("{} {label}", i + 1);
        let mut seg = link_seg(
            page,
            text,
            url.clone(),
            if i == active {
                Role::Chip(Bg::SecondaryContainer)
            } else {
                Role::Body
            },
        );
        if i == active {
            seg.text = format!(" {} ", seg.text);
        }
        segs.push(seg);
        if let Some(n) = count {
            segs.push(Seg::new(format!(" {}", compact(*n)), Role::Meta));
        }
    }
    page.line(segs);
    page.blank();
}

/// `prefix` then `segs` wrapped, with continuation lines aligned after the
/// prefix.
fn hanging(page: &mut Page, prefix: Seg, segs: Vec<Seg>) {
    let width = crate::text::width(&prefix.text) as u16;
    let start = page.lines.len();
    page.wrapped(segs, width, Surface::Page);
    if let Some(first) = page.lines.get_mut(start) {
        first.indent = 0;
        first.segs.insert(0, prefix);
    }
}

fn card_blank(page: &mut Page) {
    page.push(PageLine {
        surface: Surface::Card,
        ..PageLine::default()
    });
}

/// A comment card: author and time, then the Markdown body.
fn comment_card(
    page: &mut Page,
    author: &str,
    verb: &str,
    when: &str,
    body: &str,
    base: Option<&LinkBase>,
    now: u64,
) {
    let author_seg = link_seg(page, author.to_owned(), url::user(author), Role::Strong);
    page.push(PageLine {
        segs: vec![
            author_seg,
            Seg::new(format!(" {verb} {}", time::ago_iso(when, now)), Role::Meta),
        ],
        surface: Surface::Card,
        indent: 1,
        right: Vec::new(),
    });
    card_blank(page);
    if body.trim().is_empty() {
        page.push(PageLine {
            segs: vec![Seg::new("No description provided.", Role::Meta)],
            surface: Surface::Card,
            indent: 1,
            right: Vec::new(),
        });
    } else {
        markdown::render(page, body, base, 1, Surface::Card);
    }
    card_blank(page);
    page.blank();
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

fn state_chip(state: IssueState, is_pr: bool) -> Seg {
    match state {
        IssueState::Open => chip("Open", Bg::SuccessContainer),
        IssueState::Draft => chip("Draft", Bg::SecondaryContainer),
        IssueState::Merged => chip("Merged", Bg::TertiaryContainer),
        IssueState::NotPlanned => chip("Closed as not planned", Bg::SecondaryContainer),
        IssueState::Closed if is_pr => chip("Closed", Bg::ErrorContainer),
        IssueState::Closed => chip("Closed", Bg::TertiaryContainer),
    }
}

fn pr_state(s: PrState) -> IssueState {
    match s {
        PrState::Open => IssueState::Open,
        PrState::Draft => IssueState::Draft,
        PrState::Closed => IssueState::Closed,
        PrState::Merged => IssueState::Merged,
    }
}

// ---- repository ---------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RepoTab {
    Code,
    Issues,
    Pulls,
}

/// The repository header: name, badges, description, stats and tabs.
pub fn repo_header(page: &mut Page, repo: &RepoId, overview: Option<&RepoOverview>, tab: RepoTab) {
    let owner = link_seg(page, repo.owner.clone(), url::user(&repo.owner), Role::Link);
    let name = link_seg(page, repo.name.clone(), url::repo(repo), Role::Title);
    let mut segs = vec![owner, Seg::new(" / ", Role::Meta), name];
    if let Some(o) = overview {
        let s = &o.summary;
        segs.push(Seg::new("  ", Role::Body));
        segs.push(chip(
            if s.private { "Private" } else { "Public" },
            Bg::SecondaryContainer,
        ));
        if s.archived {
            segs.push(space());
            segs.push(chip("Archived", Bg::TertiaryContainer));
        }
    }
    page.line(segs);
    let Some(o) = overview else {
        page.blank();
        return;
    };
    if let Some(parent) = &o.parent {
        let parent_repo = RepoId::parse(parent);
        let link = parent_repo.as_ref().map(url::repo).unwrap_or_default();
        let segs = vec![
            Seg::new("forked from ", Role::Meta),
            link_seg(page, parent.clone(), link, Role::Link),
        ];
        page.line(segs);
    }
    if let Some(d) = &o.summary.description {
        page.wrapped(vec![Seg::new(d.clone(), Role::Body)], 0, Surface::Page);
    }
    let mut stats = vec![
        Seg::new(
            if o.starred { "★ " } else { "☆ " },
            if o.starred { Role::Accent } else { Role::Meta },
        ),
        Seg::new(compact(o.summary.stars), Role::Strong),
        Seg::new(" stars   ", Role::Meta),
        Seg::new("⑂ ", Role::Meta),
        Seg::new(compact(o.summary.forks), Role::Strong),
        Seg::new(" forks   ", Role::Meta),
        Seg::new("◉ ", Role::Meta),
        Seg::new(compact(o.watchers), Role::Strong),
        Seg::new(" watching", Role::Meta),
    ];
    if let Some(home) = &o.homepage {
        stats.push(Seg::new("   ", Role::Body));
        stats.push(link_seg(page, home.clone(), home.clone(), Role::Link));
    }
    page.line(stats);
    page.blank();
    let mut items = vec![("Code".to_owned(), url::repo(repo), None)];
    if o.has_issues {
        items.push(("Issues".to_owned(), url::issues(repo), Some(o.open_issues)));
    }
    items.push((
        "Pull requests".to_owned(),
        url::pulls(repo),
        Some(o.open_prs),
    ));
    let active = match tab {
        RepoTab::Code => 0,
        RepoTab::Issues => 1,
        RepoTab::Pulls => items.len() - 1,
    };
    tabs(page, &items, active);
}

/// Breadcrumbs `repo / dir / sub /` with each part a link.
fn breadcrumbs(page: &mut Page, repo: &RepoId, rev: &str, path: &str, last_is_file: bool) {
    let mut segs = vec![link_seg(
        page,
        repo.name.clone(),
        url::tree(repo, rev, ""),
        Role::Link,
    )];
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        segs.push(Seg::new(" / ", Role::Meta));
        let sub = parts[..=i].join("/");
        if i + 1 == parts.len() {
            segs.push(Seg::new(*part, Role::Strong));
            if !last_is_file {
                segs.push(Seg::new(" /", Role::Meta));
            }
        } else {
            segs.push(link_seg(
                page,
                *part,
                url::tree(repo, rev, &sub),
                Role::Link,
            ));
        }
    }
    let mut line = PageLine {
        segs,
        ..PageLine::default()
    };
    line.right = vec![Seg::new(format!("⎇ {rev}"), Role::Meta)];
    page.push(line);
    page.blank();
}

fn entry_lines(page: &mut Page, repo: &RepoId, rev: &str, path: &str, entries: &[TreeEntry]) {
    if !path.is_empty() {
        let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
        page.link_line("..", url::tree(repo, rev, parent), Role::Link, 2);
    }
    for e in entries {
        let (icon, url) = match e.kind {
            EntryKind::Dir => ("▸ ", url::tree(repo, rev, &e.path)),
            EntryKind::File => ("  ", url::blob(repo, rev, &e.path)),
            EntryKind::Symlink => ("↪ ", url::blob(repo, rev, &e.path)),
            EntryKind::Submodule => ("⊙ ", url::tree(repo, rev, &e.path)),
        };
        let name_role = if e.kind == EntryKind::Dir {
            Role::Strong
        } else {
            Role::Body
        };
        let name = link_seg(page, e.name.clone(), url, name_role);
        let mut line = PageLine {
            segs: vec![Seg::new(icon, Role::Accent), name],
            indent: 2,
            ..PageLine::default()
        };
        if let Some(bytes) = e.size {
            line.right = vec![Seg::new(size(bytes), Role::Meta)];
        }
        page.push(line);
    }
    page.blank();
}

/// The Code tab at the repository root: about, last commit, files, README.
pub fn repo_code(page: &mut Page, repo: &RepoId, overview: &RepoOverview, now: u64) {
    let rev = overview
        .default_branch
        .clone()
        .unwrap_or_else(|| "HEAD".into());
    // About: topics, language, license.
    let mut about = Vec::new();
    for t in &overview.topics {
        about.push(chip(t.clone(), Bg::PrimaryContainer));
        about.push(space());
    }
    if !about.is_empty() {
        page.wrapped(about, 0, Surface::Page);
    }
    let mut facts = Vec::new();
    if let Some(lang) = &overview.summary.language {
        facts.push(Seg::new("● ", Role::Accent));
        facts.push(Seg::new(format!("{lang}   "), Role::Body));
    }
    if let Some(license) = &overview.license {
        facts.push(Seg::new(format!("⚖ {license}   "), Role::Meta));
    }
    if overview.commits > 0 {
        facts.push(Seg::new(
            format!("{} on ", plural(overview.commits, "commit")),
            Role::Meta,
        ));
        facts.push(Seg::new(rev.clone(), Role::Code));
    }
    if !facts.is_empty() {
        page.line(facts);
        page.blank();
    }

    if overview.entries.is_empty() && overview.default_branch.is_none() {
        page.line(vec![Seg::new("This repository is empty.", Role::Meta)]);
        return;
    }
    if let Some(c) = &overview.last_commit {
        let author = link_seg(page, c.author.clone(), url::user(&c.author), Role::Strong);
        let headline = link_seg(
            page,
            c.headline.clone(),
            url::commit(repo, &c.oid),
            Role::Body,
        );
        page.push(PageLine {
            segs: vec![author, Seg::new("  ", Role::Body), headline],
            surface: Surface::Card,
            indent: 1,
            right: vec![Seg::new(
                format!(
                    "{} · {}",
                    &c.oid[..7.min(c.oid.len())],
                    time::ago_iso(&c.date, now)
                ),
                Role::Meta,
            )],
        });
        page.blank();
    }
    entry_lines(page, repo, &rev, "", &overview.entries);
    if let Some(readme) = &overview.readme {
        let dir = readme
            .path
            .rsplit_once('/')
            .map_or("", |(d, _)| d)
            .to_owned();
        let name = link_seg(
            page,
            readme.path.clone(),
            url::blob(repo, &rev, &readme.path),
            Role::Title,
        );
        page.push(PageLine {
            segs: vec![Seg::new("☰ ", Role::Meta), name],
            surface: Surface::Card,
            indent: 1,
            right: Vec::new(),
        });
        card_blank(page);
        let base = LinkBase {
            repo: repo.to_string(),
            rev,
            dir,
        };
        markdown::render(page, &readme.text, Some(&base), 1, Surface::Card);
        card_blank(page);
    }
}

/// A directory below the root.
pub fn repo_dir(
    page: &mut Page,
    repo: &RepoId,
    rev: &str,
    path: &str,
    entries: Option<&[TreeEntry]>,
) {
    breadcrumbs(page, repo, rev, path, false);
    match entries {
        Some(entries) => entry_lines(page, repo, rev, path, entries),
        None => page.line(vec![Seg::new("Loading…", Role::Meta)]),
    }
}

/// A file: highlighted with line numbers, or Markdown rendered.
pub fn file(page: &mut Page, repo: &RepoId, rev: &str, path: &str, blob: &ghtui_api::browse::Blob) {
    breadcrumbs(page, repo, rev, path, true);
    let Some(text) = &blob.text else {
        page.line(vec![Seg::new(
            format!(
                "Binary file ({}). Press o to view it on GitHub.",
                size(blob.size)
            ),
            Role::Meta,
        )]);
        return;
    };
    let lines = text.lines().count();
    page.line(vec![Seg::new(
        format!("{} · {}", plural(lines as u64, "line"), size(blob.size)),
        Role::Meta,
    )]);
    if blob.truncated {
        page.line(vec![Seg::new(
            "Large file: only the beginning is shown.",
            Role::Meta,
        )]);
    }
    page.blank();
    if path.to_ascii_lowercase().ends_with(".md") {
        let base = LinkBase {
            repo: repo.to_string(),
            rev: rev.to_owned(),
            dir: path.rsplit_once('/').map_or("", |(d, _)| d).to_owned(),
        };
        markdown::render(page, text, Some(&base), 0, Surface::Page);
        return;
    }
    let source = ghtui_diff::text::Text::new(text.as_bytes());
    let spans = ghtui_diff::highlight(ghtui_diff::Language::from_path(path), &source);
    let width = source.len().to_string().len();
    for i in 0..source.len() {
        let line = source.line(i);
        let mut segs = vec![Seg::new(
            format!("{:>width$}  ", i + 1),
            Role::Syntax(ghtui_theme::Syntax::Comment),
        )];
        let mut at = 0usize;
        for s in spans.get(i).map_or(&[][..], Vec::as_slice) {
            let (a, b) = (s.start as usize, s.end as usize);
            if a > at {
                segs.push(Seg::new(
                    &line[at..a],
                    Role::Syntax(ghtui_theme::Syntax::Default),
                ));
            }
            segs.push(Seg::new(
                &line[a..b],
                Role::Syntax(crate::diff_view::syntax_role(s.kind)),
            ));
            at = b;
        }
        if at < line.len() {
            segs.push(Seg::new(
                &line[at..],
                Role::Syntax(ghtui_theme::Syntax::Default),
            ));
        }
        page.push(PageLine {
            segs,
            surface: Surface::Code,
            ..PageLine::default()
        });
    }
}

// ---- lists --------------------------------------------------------------------------

pub fn repo_rows(page: &mut Page, repos: &[RepoSummary], now: u64) {
    for r in repos {
        let mut segs = vec![link_seg(
            page,
            r.repo.to_string(),
            url::repo(&r.repo),
            Role::Title,
        )];
        if r.private {
            segs.push(space());
            segs.push(chip("Private", Bg::SecondaryContainer));
        }
        if r.fork {
            segs.push(space());
            segs.push(chip("Fork", Bg::SecondaryContainer));
        }
        if r.archived {
            segs.push(space());
            segs.push(chip("Archived", Bg::TertiaryContainer));
        }
        page.line(segs);
        if let Some(d) = &r.description {
            page.wrapped(vec![Seg::new(d.clone(), Role::Body)], 2, Surface::Page);
        }
        let mut meta = Vec::new();
        if let Some(lang) = &r.language {
            meta.push(Seg::new("● ", Role::Accent));
            meta.push(Seg::new(format!("{lang}   "), Role::Meta));
        }
        meta.push(Seg::new(
            format!("★ {}   ⑂ {}", compact(r.stars), compact(r.forks)),
            Role::Meta,
        ));
        if let Some(p) = &r.pushed_at {
            meta.push(Seg::new(
                format!("   updated {}", time::ago_iso(p, now)),
                Role::Meta,
            ));
        }
        page.push(PageLine {
            segs: meta,
            indent: 2,
            ..PageLine::default()
        });
        page.blank();
    }
}

pub fn issue_rows(
    page: &mut Page,
    issues: &[IssueSummary],
    show_repo: bool,
    icons: Icons,
    now: u64,
) {
    for i in issues {
        let (icon, role) = issue_icon(icons, i.state, i.is_pr);
        let target = if i.is_pr {
            url::pull(&PrRef {
                repo: i.repo.clone(),
                number: i.number,
            })
        } else {
            url::issue(&i.repo, i.number)
        };
        let mut segs = vec![link_seg(page, i.title.clone(), target, Role::Strong)];
        labels(&mut segs, &i.labels);
        hanging(page, Seg::new(format!("{icon} "), role), segs);
        let place = if show_repo {
            format!("{}#{}", i.repo, i.number)
        } else {
            format!("#{}", i.number)
        };
        let mut line = PageLine {
            segs: vec![Seg::new(
                format!(
                    "{place} · {} · updated {}",
                    i.author,
                    time::ago_iso(&i.updated_at, now)
                ),
                Role::Meta,
            )],
            indent: 2,
            ..PageLine::default()
        };
        if i.comments > 0 {
            line.right = vec![Seg::new(format!("💬 {}", i.comments), Role::Meta)];
        }
        page.push(line);
    }
}

pub fn user_rows(page: &mut Page, users: &[UserSummary]) {
    for u in users {
        let mut segs = vec![link_seg(
            page,
            u.login.clone(),
            url::user(&u.login),
            Role::Title,
        )];
        if let Some(name) = &u.name {
            segs.push(Seg::new(format!("  {name}"), Role::Meta));
        }
        if u.is_org {
            segs.push(space());
            segs.push(chip("Organization", Bg::SecondaryContainer));
        }
        page.line(segs);
        if let Some(bio) = &u.bio {
            page.wrapped(vec![Seg::new(bio.clone(), Role::Body)], 2, Surface::Page);
        }
        page.blank();
    }
}

fn more(page: &mut Page, next: bool, shown: usize, total: u64) {
    if next {
        page.blank();
        page.link_line(
            format!("Load more ({shown} of {})", compact(total)),
            MORE,
            Role::Link,
            0,
        );
    }
}

/// An issue or pull-request list (a repo tab, or search results).
pub fn issue_list(
    page: &mut Page,
    query: &str,
    results: Option<&Results<IssueSummary>>,
    show_repo: bool,
    filter_key: &str,
    icons: Icons,
    now: u64,
) {
    page.line(vec![
        Seg::new("Filter: ", Role::Meta),
        Seg::new(query.to_owned(), Role::Code),
        Seg::new(format!("   {filter_key} to change"), Role::Meta),
    ]);
    page.blank();
    let Some(results) = results else {
        page.line(vec![Seg::new("Loading…", Role::Meta)]);
        return;
    };
    if results.items.is_empty() {
        page.line(vec![Seg::new("Nothing matches.", Role::Meta)]);
        return;
    }
    page.line(vec![Seg::new(plural(results.total, "result"), Role::Meta)]);
    page.blank();
    issue_rows(page, &results.items, show_repo, icons, now);
    more(
        page,
        results.next.is_some(),
        results.items.len(),
        results.total,
    );
}

// ---- search -------------------------------------------------------------------------

pub fn search(
    page: &mut Page,
    kind: SearchKind,
    query: &str,
    results: Option<&SearchResults>,
    icons: Icons,
    now: u64,
) {
    page.line(vec![
        Seg::new("Search ", Role::Title),
        Seg::new(query.to_owned(), Role::Code),
    ]);
    page.blank();
    let items = [
        (
            "Repositories".to_owned(),
            url::search(SearchKind::Repos, query),
            None,
        ),
        (
            "Issues & pull requests".to_owned(),
            url::search(SearchKind::Issues, query),
            None,
        ),
        (
            "Users".to_owned(),
            url::search(SearchKind::Users, query),
            None,
        ),
    ];
    let active = match kind {
        SearchKind::Repos => 0,
        SearchKind::Issues => 1,
        SearchKind::Users => 2,
    };
    tabs(page, &items, active);
    let Some(results) = results else {
        page.line(vec![Seg::new("Searching…", Role::Meta)]);
        return;
    };
    let (total, shown, next) = match results {
        SearchResults::Repos(r) => (r.total, r.items.len(), r.next.is_some()),
        SearchResults::Issues(r) => (r.total, r.items.len(), r.next.is_some()),
        SearchResults::Users(r) => (r.total, r.items.len(), r.next.is_some()),
    };
    page.line(vec![Seg::new(plural(total, "result"), Role::Meta)]);
    page.blank();
    match results {
        SearchResults::Repos(r) => repo_rows(page, &r.items, now),
        SearchResults::Issues(r) => issue_rows(page, &r.items, true, icons, now),
        SearchResults::Users(r) => user_rows(page, &r.items),
    }
    more(page, next, shown, total);
}

// ---- issue ---------------------------------------------------------------------------

pub fn issue(page: &mut Page, d: &IssueDetail, comment_key: &str, now: u64) {
    repo_line(page, &d.repo);
    page.wrapped(
        vec![
            Seg::new(d.title.clone(), Role::Title),
            Seg::new(format!("  #{}", d.number), Role::Meta),
        ],
        0,
        Surface::Page,
    );
    let author = link_seg(page, d.author.clone(), url::user(&d.author), Role::Strong);
    let mut segs = vec![
        state_chip(d.state, false),
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
    labels(&mut segs, &d.labels);
    page.wrapped(segs, 0, Surface::Page);
    if !d.assignees.is_empty() {
        let mut segs = vec![Seg::new("Assignees: ", Role::Meta)];
        for (i, a) in d.assignees.iter().enumerate() {
            if i > 0 {
                segs.push(Seg::new(", ", Role::Meta));
            }
            segs.push(link_seg(page, a.clone(), url::user(a), Role::Link));
        }
        page.line(segs);
    }
    page.blank();
    let base = LinkBase {
        repo: d.repo.to_string(),
        rev: "HEAD".into(),
        dir: String::new(),
    };
    comment_card(
        page,
        &d.author,
        "commented",
        &d.created_at,
        &d.body,
        Some(&base),
        now,
    );
    for c in &d.comments {
        comment_card(
            page,
            &c.author,
            "commented",
            &c.created_at,
            &c.body,
            Some(&base),
            now,
        );
    }
    page.line(vec![Seg::new(
        format!("{comment_key} to comment · o to open on GitHub"),
        Role::Meta,
    )]);
}

fn repo_line(page: &mut Page, repo: &RepoId) {
    let owner = link_seg(page, repo.owner.clone(), url::user(&repo.owner), Role::Link);
    let name = link_seg(page, repo.name.clone(), url::repo(repo), Role::Link);
    page.line(vec![owner, Seg::new(" / ", Role::Meta), name]);
    page.blank();
}

// ---- pull request -------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrTab {
    Conversation,
    Commits,
}

fn pr_header(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Option<&PrActivity>,
    tab: Option<PrTab>,
    now: u64,
) {
    repo_line(page, &pr.repo);
    let s = &d.summary;
    page.wrapped(
        vec![
            Seg::new(s.title.clone(), Role::Title),
            Seg::new(format!("  #{}", pr.number), Role::Meta),
        ],
        0,
        Surface::Page,
    );
    let author = link_seg(page, s.author.clone(), url::user(&s.author), Role::Strong);
    let head = match &d.head_repo {
        Some(repo) if *repo != pr.repo.to_string() => {
            format!("{}:{}", repo.split('/').next().unwrap_or(repo), d.head_ref)
        }
        _ => d.head_ref.clone(),
    };
    let mut segs = vec![
        state_chip(pr_state(s.state), true),
        Seg::new("  ", Role::Body),
        author,
        Seg::new(" wants to merge into ", Role::Meta),
        Seg::new(d.base_ref.clone(), Role::Code),
        Seg::new(" from ", Role::Meta),
        Seg::new(head, Role::Code),
    ];
    match s.review {
        Some(ReviewDecision::Approved) => {
            segs.push(space());
            segs.push(chip("Approved", Bg::SuccessContainer));
        }
        Some(ReviewDecision::ChangesRequested) => {
            segs.push(space());
            segs.push(chip("Changes requested", Bg::ErrorContainer));
        }
        _ => {}
    }
    match s.checks {
        Some(ChecksState::Passing) => {
            segs.push(space());
            segs.push(chip("✓ Checks", Bg::SuccessContainer));
        }
        Some(ChecksState::Failing) => {
            segs.push(space());
            segs.push(chip("✗ Checks", Bg::ErrorContainer));
        }
        Some(ChecksState::Pending) => {
            segs.push(space());
            segs.push(chip("● Checks", Bg::SecondaryContainer));
        }
        None => {}
    }
    labels(&mut segs, &d.labels);
    page.wrapped(segs, 0, Surface::Page);
    page.line(vec![
        Seg::new(format!("+{}", s.additions), Role::Added),
        Seg::new(" ", Role::Body),
        Seg::new(format!("−{}", s.deletions), Role::Removed),
        Seg::new(
            format!(
                " · opened {} · updated {}",
                time::ago_iso(&d.created_at, now),
                time::ago_iso(&s.updated_at, now)
            ),
            Role::Meta,
        ),
    ]);
    page.blank();
    if let Some(tab) = tab {
        let commits = activity.map(|a| a.commits.len() as u64);
        tabs(
            page,
            &[
                (
                    "Conversation".to_owned(),
                    url::pull(pr),
                    activity.map(|a| a.comments.len() as u64),
                ),
                ("Commits".to_owned(), url::pull_tab(pr, "commits"), commits),
                (
                    "Files changed".to_owned(),
                    url::pull_tab(pr, "files"),
                    Some(d.changed_files),
                ),
            ],
            match tab {
                PrTab::Conversation => 0,
                PrTab::Commits => 1,
            },
        );
    }
}

pub fn pr_conversation(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Option<&PrActivity>,
    comment_key: &str,
    now: u64,
) {
    pr_header(page, pr, d, activity, Some(PrTab::Conversation), now);
    let base = LinkBase {
        repo: pr.repo.to_string(),
        rev: d.head_oid.clone(),
        dir: String::new(),
    };
    comment_card(
        page,
        &d.summary.author,
        "commented",
        &d.created_at,
        &d.body,
        Some(&base),
        now,
    );
    let Some(a) = activity else {
        page.line(vec![Seg::new("Loading the conversation…", Role::Meta)]);
        return;
    };
    // Comments and reviews in time order.
    enum Item<'a> {
        Comment(&'a ghtui_api::browse::Comment),
        Review(&'a ghtui_api::browse::ReviewSummary),
    }
    let mut items: Vec<(&str, Item<'_>)> = a
        .comments
        .iter()
        .map(|c| (c.created_at.as_str(), Item::Comment(c)))
        .chain(
            a.reviews
                .iter()
                .map(|r| (r.submitted_at.as_str(), Item::Review(r))),
        )
        .collect();
    items.sort_by(|x, y| x.0.cmp(y.0));
    for (_, item) in items {
        match item {
            Item::Comment(c) => comment_card(
                page,
                &c.author,
                "commented",
                &c.created_at,
                &c.body,
                Some(&base),
                now,
            ),
            Item::Review(r) if r.body.trim().is_empty() => {
                let author = link_seg(page, r.author.clone(), url::user(&r.author), Role::Strong);
                let role = match r.state.as_str() {
                    "approved" => Role::Success,
                    "requested changes" => Role::Error,
                    _ => Role::Meta,
                };
                page.line(vec![
                    Seg::new("● ", role),
                    author,
                    Seg::new(
                        format!(" {} {}", r.state, time::ago_iso(&r.submitted_at, now)),
                        Role::Meta,
                    ),
                ]);
                page.blank();
            }
            Item::Review(r) => comment_card(
                page,
                &r.author,
                &r.state,
                &r.submitted_at,
                &r.body,
                Some(&base),
                now,
            ),
        }
    }
    let files = link_seg(
        page,
        format!("Files changed ({})", d.changed_files),
        url::pull_tab(pr, "files"),
        Role::Link,
    );
    page.line(vec![
        files,
        Seg::new(format!("   {comment_key} to comment"), Role::Meta),
    ]);
}

pub fn pr_commits(
    page: &mut Page,
    pr: &PrRef,
    d: &PrDetail,
    activity: Option<&PrActivity>,
    now: u64,
) {
    pr_header(page, pr, d, activity, Some(PrTab::Commits), now);
    let Some(a) = activity else {
        page.line(vec![Seg::new("Loading commits…", Role::Meta)]);
        return;
    };
    for c in &a.commits {
        let headline = link_seg(
            page,
            c.headline.clone(),
            url::commit(&pr.repo, &c.oid),
            Role::Strong,
        );
        let mut line = PageLine {
            segs: vec![Seg::new("● ", Role::Meta), headline],
            ..PageLine::default()
        };
        line.right = vec![Seg::new(c.oid[..7.min(c.oid.len())].to_owned(), Role::Code)];
        page.push(line);
        page.push(PageLine {
            segs: vec![Seg::new(
                format!("{} committed {}", c.author, time::ago_iso(&c.date, now)),
                Role::Meta,
            )],
            indent: 2,
            ..PageLine::default()
        });
    }
}

// ---- profile -------------------------------------------------------------------------------

pub fn profile(page: &mut Page, p: &Profile, now: u64) {
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
        page.wrapped(vec![Seg::new(bio.clone(), Role::Body)], 0, Surface::Page);
    }
    let mut facts = Vec::new();
    if let (Some(followers), Some(following)) = (p.followers, p.following) {
        facts.push(Seg::new(
            format!(
                "{} followers · {} following",
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
        facts.push(Seg::new("   ", Role::Meta));
        facts.push(link_seg(page, site.clone(), site.clone(), Role::Link));
    }
    if !facts.is_empty() {
        page.wrapped(facts, 0, Surface::Page);
    }
    page.blank();
    if !p.pinned.is_empty() {
        page.line(vec![Seg::new("Pinned", Role::Title)]);
        page.blank();
        repo_rows(page, &p.pinned, now);
    }
    page.line(vec![
        Seg::new("Repositories", Role::Title),
        Seg::new(format!("  {}", compact(p.repo_count)), Role::Meta),
    ]);
    page.blank();
    repo_rows(page, &p.repos, now);
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
    let pr_rows = |page: &mut Page, prs: &[PrSummary]| {
        for p in prs {
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
            };
            issue_rows(page, std::slice::from_ref(&summary), true, icons, now);
        }
    };
    match inbox {
        Some(inbox) => {
            let all = link_seg(
                page,
                "see all",
                url::search(
                    SearchKind::Issues,
                    "is:open is:pr review-requested:@me archived:false",
                ),
                Role::Link,
            );
            page.line(vec![
                Seg::new("Review requested", Role::Title),
                Seg::new(
                    format!(
                        "  {}   ",
                        inbox
                            .review_requested_total
                            .max(inbox.review_requested.len() as u64)
                    ),
                    Role::Meta,
                ),
                all,
            ]);
            page.blank();
            if inbox.review_requested.is_empty() {
                page.line(vec![Seg::new(
                    "Nothing is waiting for your review.",
                    Role::Meta,
                )]);
            }
            pr_rows(page, &inbox.review_requested);
            page.blank();
            let all = link_seg(
                page,
                "see all",
                url::search(
                    SearchKind::Issues,
                    "is:open is:pr author:@me archived:false",
                ),
                Role::Link,
            );
            page.line(vec![
                Seg::new("Your pull requests", Role::Title),
                Seg::new(
                    format!(
                        "  {}   ",
                        inbox.authored_total.max(inbox.authored.len() as u64)
                    ),
                    Role::Meta,
                ),
                all,
            ]);
            page.blank();
            if inbox.authored.is_empty() {
                page.line(vec![Seg::new(
                    "You have no open pull requests.",
                    Role::Meta,
                )]);
            }
            pr_rows(page, &inbox.authored);
            page.blank();
        }
        None => page.line(vec![Seg::new("Loading your pull requests…", Role::Meta)]),
    }
    page.blank();
    let mut title = vec![Seg::new("Your repositories", Role::Title)];
    if let Some(login) = viewer {
        title.push(Seg::new("   ", Role::Body));
        title.push(link_seg(page, "see all", url::user(login), Role::Link));
    }
    page.line(title);
    page.blank();
    match repos {
        Some(repos) => repo_rows(page, repos, now),
        None => page.line(vec![Seg::new("Loading…", Role::Meta)]),
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
            license: Some("MIT".into()),
            topics: vec!["tui".into()],
            default_branch: Some("main".into()),
            last_commit: None,
            commits: 10,
            parent: None,
            starred: false,
            id: "R_1".into(),
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
        repo_header(&mut page, &repo, Some(&overview), RepoTab::Code);
        repo_code(&mut page, &repo, &overview, 0);
        for url in [
            "https://github.com/o/r/tree/main/src",
            "https://github.com/o/r/blob/main/README.md",
            "https://github.com/o/r/blob/main/docs/intro.md",
            "https://github.com/o/r/issues",
            "https://github.com/o",
        ] {
            assert!(
                page.links.iter().any(|l| l == url),
                "{url} missing: {:?}",
                page.links
            );
        }
        let text: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert!(text.iter().any(|l| l.contains("1.2k stars")), "{text:?}");
        assert!(text.iter().any(|l| l.contains("# Hello")), "{text:?}");
    }
}
