//! A repository: its code and directories, releases, tags and branches,
//! and comparisons.

use super::*;
use std::fmt::Write as _;

/// The repository's title: `owner / name  Public` with Star, Fork and Watch
/// buttons on the right, as on GitHub.
pub fn repo_title(page: &mut Page, repo: &RepoId, overview: Option<&RepoOverview>) {
    let owner = login_seg(page, &repo.owner, Role::Link);
    let name = link_seg(page, repo.name.clone(), url::repo(repo), Role::Title);
    let mut segs = vec![owner, meta(" / "), name];
    let mut right = Vec::new();
    if let Some(o) = overview {
        let s = &o.summary;
        segs.push(Seg::new("  ", Role::Body));
        let visibility = if s.private { "Private" } else { "Public" };
        segs.push(chip(visibility, Bg::SecondaryContainer));
        if s.archived {
            tag(&mut segs, "Archived", Bg::TertiaryContainer);
        }
        let star = if o.starred { "★ Starred" } else { "☆ Star" };
        let star = format!("{star}  {}", compact(s.stars));
        right.push(button(page, star, Link::Star));
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
        page.line(vec![meta("forked from "), parent]);
    }
    page.blank();
}

/// The About facts: description, homepage, topics, counts.
pub(super) fn about(page: &mut Page, repo: &RepoId, o: &RepoOverview, compact_layout: bool) {
    if !compact_layout {
        page.line(vec![strong("About")]);
        page.blank();
    }
    match &o.summary.description {
        Some(d) => say(page, d.clone(), Role::Body),
        None if !compact_layout => {
            page.line(vec![meta("No description, website, or topics provided.")]);
        }
        None => {}
    }
    if let Some(home) = &o.homepage {
        let link = link_seg(page, home.clone(), home.clone(), Role::Link);
        page.wrapped(vec![meta("↗ "), link], 0, Frame::None);
    }
    if !o.topics.is_empty() {
        let mut topics = Vec::new();
        for t in &o.topics {
            let (text, target) = (format!(" {t} "), url::topic(&t.to_lowercase()));
            let topic = link_seg(page, text, target, Role::Chip(Bg::PrimaryContainer));
            topics.extend([topic, space()]);
        }
        more_chips(&mut topics, &o.topics);
        page.blank();
        page.wrapped(topics, 0, Frame::None);
    }
    let mut facts = Vec::new();
    if let Some(license) = &o.license {
        facts.push(meta(format!("⚖ {license} license")));
    }
    let (stars, forks) = (o.summary.stars, o.summary.forks);
    for (icon, n, what, tab) in [
        ("☆", stars, noun(stars, "star"), "stargazers"),
        ("◉", o.watchers, "watching".to_owned(), "watchers"),
        ("⑂", forks, noun(forks, "fork"), "forks"),
    ] {
        let text = format!("{icon} {} {what}", compact(n));
        let target = format!("{}/{tab}", url::repo(repo));
        facts.push(link_seg(page, text, target, Role::Meta));
    }
    if let Some(language) = &o.summary.language {
        facts.push(meta(format!("● {language}")));
    }
    let packages = format!("{}/packages", url::repo(repo));
    facts.push(link_seg(page, "⬢ Packages", packages, Role::Meta));
    page.blank();
    if compact_layout {
        let mut segs = Vec::new();
        for fact in facts {
            sep(&mut segs, "   ");
            segs.push(fact);
        }
        page.wrapped(segs, 0, Frame::None);
    } else {
        for fact in facts {
            page.wrapped(vec![fact], 0, Frame::None);
        }
    }
}

/// `⎇ main ▾   ⌕ Go to file  t` above the file list or file.
pub(super) fn code_toolbar(
    page: &mut Page,
    repo: &RepoId,
    rev: &str,
    path: &str,
    is_file: bool,
    top: Option<&RepoOverview>,
    keys: Keys<'_>,
) {
    // At the top of a repository: its commits, branches and tags.
    let commits = top.map_or(0, |o| o.commits);
    let refs = top.filter(|o| o.branches > 0).map(|o| (o.branches, o.tags));
    // A commit's ID is shown short, as GitHub's branch button shows it.
    let shown = match crate::git::Oid::parse(rev) {
        Ok(_) => short_sha(rev),
        Err(_) => rev,
    };
    let mut segs = vec![
        button(page, format!("⎇ {shown} ▾"), Link::Branch),
        Seg::new("  ", Role::Body),
    ];
    if let Some((branches, tags)) = refs {
        let base = url::repo(repo);
        for (text, tab) in [
            (format!("⎇ {}", plural(branches, "branch")), "branches"),
            (format!("◇ {}", plural(tags, "tag")), "tags"),
        ] {
            segs.push(link_seg(page, text, format!("{base}/{tab}"), Role::Meta));
            segs.push(Seg::new("  ", Role::Body));
        }
    }
    if is_file {
        segs.extend(crumbs(page, repo, rev, path, true));
    } else if !path.is_empty() {
        segs.extend(crumbs(page, repo, rev, path, false));
        segs.push(Seg::new("  ", Role::Body));
    }
    let find = go_to_file(page, keys);
    let mut right = Vec::new();
    if commits > 0 {
        let text = format!("◷ {}", plural(commits, "commit"));
        let history = link_seg(page, text, url::commits(repo, rev, path), Role::Meta);
        right.extend([history, space()]);
    }
    right.extend(find);
    page.add(Frame::None, 0, segs, right);
}

/// `repo / dir / sub /` with each part a link.
pub(super) fn crumbs(
    page: &mut Page,
    repo: &RepoId,
    rev: &str,
    path: &str,
    last_is_file: bool,
) -> Vec<Seg> {
    let root = url::tree(repo, rev, "");
    let mut segs = vec![link_seg(page, &repo.name, root, Role::Link)];
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    for (i, part) in parts.iter().enumerate() {
        segs.push(meta(" / "));
        if i + 1 == parts.len() {
            segs.push(strong(*part));
            if !last_is_file {
                segs.push(meta(" /"));
            }
        } else {
            let sub = url::tree(repo, rev, &parts.get(..=i).unwrap_or_default().join("/"));
            segs.push(link_seg(page, *part, sub, Role::Link));
        }
    }
    segs
}

/// The file list box.
pub(super) fn file_box(
    page: &mut Page,
    dir: Listing<'_>,
    title: (Vec<Seg>, Vec<Seg>),
    cx: PageCtx<'_>,
) {
    let Listing {
        repo,
        rev,
        path,
        entries,
        commits,
    } = dir;
    let PageCtx { icons, now, .. } = cx;
    let Some(entries) = entries.show(page, "the directory") else { return };
    page.box_top(title.0, title.1);
    // GitHub's columns: name, the latest commit's message, its age.
    let name_w = entries
        .iter()
        .map(|e| width(&e.name))
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
        [] => empty_row(page, "This directory is empty."),
        entries => {
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
                    let name = truncate(&e.name, name_w - 4);
                    let pad = name_w.saturating_sub(width(&name) + 2);
                    let mut segs = vec![
                        Seg::new(format!("{icon} "), icon_role),
                        Seg::linked(name, role, link),
                        Seg::new(" ".repeat(pad), Role::Body),
                    ];
                    let mut right = Vec::new();
                    if let Some(c) = commits.and_then(|m| m.get(&e.name)) {
                        let commit = page.link(url::commit(repo, &c.oid));
                        segs.push(Seg::linked(c.headline.clone(), Role::Meta, commit));
                        right.push(meta(time::ago_iso(&c.date, now)));
                    }
                    page.box_line(segs, right, 0);
                });
            }
            let past = entries
                .len()
                .saturating_sub(crate::api::browse::LAST_COMMIT_ENTRIES);
            if commits.is_some() && past > 0 {
                let text = format!(
                    "Latest commits are for the first {} entries; not the last {past}",
                    crate::api::browse::LAST_COMMIT_ENTRIES
                );
                body(page, vec![meta(text)]);
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
    readme: Fetched<'_, Option<Box<Readme>>>,
    cx: PageCtx<'_>,
) {
    let PageCtx { keys, now, .. } = cx;
    if !page.build_aside(|a| about(a, repo, o, false)) {
        about(page, repo, o, true);
        page.blank();
    }
    if o.entries.is_empty() && o.default_branch.is_none() {
        let title = strong("This repository is empty.");
        empty_box(page, title, "Nothing has been pushed yet.");
        return;
    }
    let rev = o.default_branch.clone().unwrap_or_else(|| "HEAD".into());
    code_toolbar(page, repo, &rev, "", false, Some(o), keys);
    let title = match &o.last_commit {
        Some(c) => {
            let author = person_seg(page, &c.author, Role::Strong);
            let commit = url::commit(repo, &c.oid);
            let headline = link_seg(page, &c.headline, commit, Role::Body);
            let when = format!("{} · {}", short_sha(&c.oid), time::ago_iso(&c.date, now));
            let title = vec![author, Seg::new("  ", Role::Body), headline];
            (title, vec![meta(when)])
        }
        None => (vec![strong("Files")], Vec::new()),
    };
    let dir = Listing {
        repo,
        rev: &rev,
        path: "",
        entries: Fetched::ready(&o.entries),
        commits,
    };
    file_box(page, dir, title, cx);
    if let Some(Some(readme)) = readme.show(page, "the README") {
        let file = url::blob(repo, &rev, &readme.path);
        let name = link_seg(page, format!("☰ {}", readme.path), file, Role::Strong);
        page.box_top(vec![name], Vec::new());
        let base = LinkBase::new(repo, rev, &readme.path);
        markdown::render(page, &readme.text, Some(&base), Frame::Body);
        page.box_bottom();
    }
}

/// A directory below the root (or the root at another branch).
pub fn repo_dir(page: &mut Page, dir: Listing<'_>, cx: PageCtx<'_>) {
    let Listing {
        repo, rev, path, ..
    } = dir;
    code_toolbar(page, repo, rev, path, false, None, cx.keys);
    let title = if path.is_empty() {
        format!("Files on {rev}")
    } else {
        path.rsplit('/').next().unwrap_or(path).to_owned()
    };
    file_box(page, dir, (vec![strong(title)], Vec::new()), cx);
}

/// A commit's short ID, linked to it.
pub(super) fn sha_seg(page: &mut Page, repo: &RepoId, oid: &str, role: Role) -> Seg {
    link_seg(page, short_sha(oid), url::commit(repo, oid), role)
}

/// A branch, linked to its code.
pub(super) fn branch_seg(page: &mut Page, repo: &RepoId, branch: &str, role: Role) -> Seg {
    link_seg(page, branch, url::tree(repo, branch, ""), role)
}

pub(super) fn release_chips(segs: &mut Vec<Seg>, r: &Release) {
    for (on, text, bg) in [
        (r.latest, "Latest", Bg::SuccessContainer),
        (r.prerelease, "Pre-release", Bg::TertiaryContainer),
        (r.draft, "Draft", Bg::SecondaryContainer),
    ] {
        if on {
            tag(segs, text, bg);
        }
    }
}

/// Who released it and when.
pub(super) fn release_meta(r: &Release, now: u64) -> String {
    let mut subline = r.tag.clone();
    if let Some(author) = &r.author {
        let _ = write!(subline, " · {author}");
    }
    match &r.published_at {
        Some(at) => _ = write!(subline, " released {}", time::ago_iso(at, now)),
        None => subline.push_str(" · not published"),
    }
    subline
}

/// A repository's releases, newest first, each linked to its page.
pub fn releases(page: &mut Page, repo: &RepoId, list: Fetched<'_, Results<Release>>, now: u64) {
    paged_list(page, "Releases", list, "No releases yet.", |page, r| {
        item(page, url::release(repo, &r.tag), |page, link| {
            let mut segs = vec![Seg::linked(r.name.clone(), Role::Link, link)];
            release_chips(&mut segs, r);
            body(page, segs);
            body(page, vec![meta(release_meta(r, now))]);
        });
    });
}

/// A release: its notes and its assets, each with its size and how
/// often it was downloaded (`↵` on one saves it).
pub fn release(page: &mut Page, repo: &RepoId, r: &Release, now: u64) {
    let mut title = vec![Seg::new(r.name.clone(), Role::Title)];
    release_chips(&mut title, r);
    page.wrapped(title, 0, Frame::None);
    let tag = link_seg(page, r.tag.clone(), url::tree(repo, &r.tag, ""), Role::Code);
    let mut subline = vec![meta("◇ "), tag];
    if let Some(author) = &r.author {
        subline.push(meta(" · "));
        subline.push(login_seg(page, author, Role::Strong));
    }
    if let Some(at) = &r.published_at {
        subline.push(meta(format!(" released {}", time::ago_iso(at, now))));
    }
    page.wrapped(subline, 0, Frame::None);
    page.blank();
    match &r.notes {
        Some(notes) => {
            let base = LinkBase::new(repo, &r.tag, "");
            markdown::render(page, notes, Some(&base), Frame::None);
        }
        None => page.line(vec![meta("No release notes.")]),
    }
    page.blank();
    let hint = if r.assets.is_empty() {
        Vec::new()
    } else {
        vec![meta("↵ saves one")]
    };
    counted_box(page, "Assets", hint, &r.assets, "No assets.", |page, a| {
        let (bytes, downloads) = (size(a.size), plural(a.downloads, "download"));
        let right = vec![meta(format!("{bytes} · {downloads}"))];
        item(page, Link::Asset(a.name.clone()), |page, link| {
            link_line(page, a.name.clone(), Role::Link, link, right);
        });
    });
}

/// A repository's tags, each linked to its code.
pub fn tags(page: &mut Page, repo: &RepoId, list: Fetched<'_, Results<TagInfo>>, now: u64) {
    paged_list(page, "Tags", list, "No tags yet.", |page, t| {
        item(page, url::tree(repo, &t.name, ""), |page, link| {
            let mut right = Vec::new();
            if let Some(date) = &t.date {
                right.push(meta(time::ago_iso(date, now)));
            }
            if let Some(oid) = &t.oid {
                right.push(Seg::new(format!("  {}", short_sha(oid)), Role::Code));
            }
            link_line(page, t.name.clone(), Role::Link, link, right);
        });
    });
}

/// A repository's branches by name: each one's latest commit and pull
/// request.
pub fn branches(
    page: &mut Page,
    repo: &RepoId,
    list: Fetched<'_, Results<BranchInfo>>,
    icons: Icons,
    now: u64,
) {
    paged_list(page, "Branches", list, "No branches.", |page, b| {
        item(page, url::tree(repo, &b.name, ""), |page, link| {
            let mut segs = vec![Seg::linked(b.name.clone(), Role::Link, link)];
            if b.default {
                tag(&mut segs, "Default", Bg::SecondaryContainer);
            }
            let mut right = Vec::new();
            if let Some((number, state)) = b.pr {
                let (icon, role) = issue_icon(icons, Status::Pr(state));
                let pr = url::pull(&PrRef {
                    repo: repo.clone(),
                    number,
                });
                let pr = link_seg(page, format!("#{number}"), pr, Role::Link);
                right.extend([Seg::new(format!("{icon} "), role), pr, meta("  ")]);
            }
            if let Some(date) = &b.date {
                right.push(meta(time::ago_iso(date, now)));
            }
            page.box_line(segs, right, 0);
            let mut subline = Vec::new();
            if let Some(oid) = &b.oid {
                subline.push(sha_seg(page, repo, oid, Role::Code));
                subline.push(meta(" "));
            }
            let what = [b.headline.as_deref(), b.author.as_ref().map(Person::name)];
            let text = what.into_iter().flatten().collect::<Vec<_>>().join(" · ");
            subline.push(meta(text));
            body(page, subline);
        });
    });
}

/// Two revisions compared: how they differ, a link to the files changed,
/// and the commits from one to the other, oldest first.
pub fn compare(page: &mut Page, repo: &RepoId, spec: &str, c: &Comparison, now: u64) {
    // One revision is compared with the default branch.
    let (base, head) = spec
        .split_once("...")
        .or_else(|| spec.split_once(".."))
        .unwrap_or(("the default branch", spec));
    let dots = if spec.contains("..") && !spec.contains("...") {
        ".."
    } else {
        "..."
    };
    let segs = vec![
        Seg::new("Comparing ", Role::Title),
        Seg::new(base.to_owned(), Role::Code),
        meta(dots),
        Seg::new(head.to_owned(), Role::Code),
    ];
    page.wrapped(segs, 0, Frame::None);
    let status = match (c.ahead, c.behind) {
        (0, 0) => "These are identical.".to_owned(),
        (0, _) => format!("{head} is {} behind {base}.", plural(c.behind, "commit")),
        (_, 0) => format!("{head} is {} ahead of {base}.", plural(c.ahead, "commit")),
        _ => format!(
            "{head} is {} ahead of and {} behind {base}.",
            plural(c.ahead, "commit"),
            plural(c.behind, "commit")
        ),
    };
    say(page, status, Role::Meta);
    let files = format!("{}#files", url::compare(repo, &c.from, &c.to));
    let changed = match c.files {
        1 => "1 file changed".to_owned(),
        // GitHub lists the first 300; the diff has them all.
        n if c.files_capped => format!("{n}+ files changed"),
        n => format!("{n} files changed"),
    };
    let mut segs = vec![link_seg(page, format!("± {changed}"), files, Role::Link)];
    segs.push(meta("  "));
    segs.extend(changes(c.additions, c.deletions));
    if c.files_capped {
        // The lines counted are those of the files GitHub listed.
        segs.push(meta(format!(" in the first {}", c.files)));
    }
    page.wrapped(segs, 0, Frame::None);
    page.blank();
    if c.commits.is_empty() {
        empty_box(page, meta("◷ Commits"), "No commits between these.");
        return;
    }
    commit_rows(page, repo, None, &c.commits, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A comparison's line counts come from the files GitHub listed, so
    /// past 300 files they say so.
    #[test]
    fn a_capped_comparison_says_its_counts_are_partial() {
        let c = Comparison {
            ahead: 1,
            from: "a".into(),
            to: "b".into(),
            files: 300,
            files_capped: true,
            additions: 10,
            deletions: 2,
            ..Default::default()
        };
        let mut page = Page::new(100);
        compare(&mut page, &RepoId::new("o", "r"), "a...b", &c, 0);
        let text: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert!(
            text.iter()
                .any(|l| l.contains("300+ files changed  +10 −2 in the first 300")),
            "{text:#?}"
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
                ..Default::default()
            },
            watchers: 5,
            open_issues: 3,
            open_prs: 1,
            closed_issues: 9,
            closed_prs: 4,
            license: Some("MIT".into()),
            topics: vec!["tui".into()].into(),
            default_branch: Some("main".into()),
            commits: 10,
            id: crate::api::model::NodeId::new("R_1"),
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
            ..Default::default()
        };
        let readme = Some(Box::new(Readme {
            path: "README.md".into(),
            text: "# Hello\n\nSee [docs](docs/intro.md).".into(),
        }));
        let mut page = Page::new(80);
        repo_title(&mut page, &repo, Some(&overview));
        let readme = Fetched::ready(&readme);
        repo_code(
            &mut page,
            &repo,
            &overview,
            None,
            readme,
            PageCtx::default(),
        );
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
}
