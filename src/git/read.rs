//! A repository read at a commit, as GitHub's pages show it: a directory,
//! a file, every path, a history, a commit, its blame, a comparison, the
//! latest commit of each entry, branches and tags, and a code search.
//!
//! Everything here reads at a commit ID ([`Oid`]): which commit a name
//! (`main`, `v1.0`) means is [`crate::source`]'s to decide, against
//! GitHub. In a partial clone, what a read needs is fetched in one batch
//! first, so git never falls back to fetching blobs one at a time; the
//! limits below keep a huge repository from turning a page into a
//! download.

use std::collections::{HashMap, HashSet};

use tokio::io::{AsyncBufReadExt, BufReader};

use crate::api::browse::Person;
use crate::api::browse::{
    Blame, BlameRange, Blob, CodeHit, CommitDetail, CommitInfo, Comparison, EntryKind, Results,
    TagInfo, TreeEntry,
};
use crate::api::model::{Capped, RepoId};
use crate::git::files::{ChangedFile, MODE_SUBMODULE, MODE_SYMLINK, ZERO_OID};
use crate::git::repo::{EMPTY_TREE, Repo};
use crate::git::{GitError, Oid, piped};

/// The most of a file's text shown; past it, the start is, as GitHub
/// shows a big file.
pub const TEXT_MAX: usize = 1024 * 1024;
/// The most paths "Go to file" lists.
pub const FILES_MAX: usize = 250_000;
/// The most files a commit or comparison counts its changed lines in (it
/// needs their contents).
pub const STAT_FILES_MAX: usize = 3_000;
/// The most versions of a file blame reads.
pub const BLAME_VERSIONS_MAX: usize = 2_000;
/// The most files a code search reads in a partial clone (in yours, any).
pub const GREP_FETCH_MAX: usize = 25_000;
/// The most matching lines kept for each file a code search finds.
pub const GREP_LINES: usize = 3;
/// Commits on a page of a history, as GitHub's.
pub const HISTORY_PAGE: usize = 30;

/// Why git didn't answer: a limit above, said plainly, or git's failure.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{0}")]
    Limit(String),
    #[error(transparent)]
    Git(#[from] GitError),
}

type Read<T> = Result<T, ReadError>;

/// The fields of `--format=` for a [`CommitInfo`]; see [`commit_info`].
const INFO: &str = "%H%x1f%an%x1f%ae%x1f%ct%x1f%s";

/// A commit from [`INFO`]'s fields.
fn commit_info(line: &str) -> Option<CommitInfo> {
    let mut f = line.split('\u{1f}');
    let (oid, name, email, time, subject) = (f.next()?, f.next()?, f.next()?, f.next()?, f.next());
    Some(CommitInfo {
        oid: Oid::parse(oid.trim()).ok()?.to_string(),
        headline: subject.unwrap_or_default().to_owned(),
        author: Person::from_git(name, email),
        date: iso(time),
    })
}

/// Unix seconds as GitHub writes times.
fn iso(secs: &str) -> String {
    crate::ui::time::iso8601(secs.trim().parse().unwrap_or(0))
}

/// `<at>:<path>`: the root for an empty path.
fn spot(at: &Oid, path: &str) -> String {
    format!("{at}:{}", path.trim_matches('/'))
}

/// The directory `path` at `at`, directories first, as GitHub lists it;
/// `None` when there's no directory there.
pub async fn tree(repo: &Repo, at: &Oid, path: &str) -> Read<Option<Vec<TreeEntry>>> {
    let spot = spot(at, path);
    if !path.trim_matches('/').is_empty()
        && entry(repo, at, path)
            .await?
            .is_none_or(|(kind, _)| kind != "tree")
    {
        return Ok(None);
    }
    let out = repo.run(&["ls-tree", "-z", &spot]).await?;
    let dir = path.trim_matches('/');
    let mut entries: Vec<TreeEntry> = (out.split('\0'))
        .filter_map(|line| {
            let (meta, name) = line.split_once('\t')?;
            let mut meta = meta.split(' ');
            let mode = u32::from_str_radix(meta.next()?, 8).ok()?;
            let kind = match meta.next()? {
                "tree" => EntryKind::Dir,
                "commit" => EntryKind::Submodule,
                _ if mode == MODE_SYMLINK => EntryKind::Symlink,
                _ => EntryKind::File,
            };
            Some(TreeEntry {
                name: name.to_owned(),
                path: if dir.is_empty() {
                    name.to_owned()
                } else {
                    format!("{dir}/{name}")
                },
                kind,
                // A partial clone may not have the file to measure.
                size: None,
            })
        })
        .collect();
    entries.sort_by_cached_key(|e| (e.kind != EntryKind::Dir, e.name.to_lowercase()));
    Ok(Some(entries))
}

/// What `path` is at `at` (`blob`, `tree`, `commit`) and its object ID,
/// from its directory's listing: without reading the object, which a
/// partial clone may not have.
async fn entry(repo: &Repo, at: &Oid, path: &str) -> Read<Option<(String, String)>> {
    let path = path.trim_matches('/');
    let out = repo.run(&["ls-tree", "-z", at, "--", path]).await?;
    Ok(out.split('\0').find_map(|line| {
        let (meta, name) = line.split_once('\t')?;
        let mut meta = meta.split(' ');
        let (_, kind, oid) = (meta.next()?, meta.next()?, meta.next()?);
        (name == path).then(|| (kind.to_owned(), oid.to_owned()))
    }))
}

/// The file `path` at `at` (fetched first, in a partial clone, if it
/// isn't here); `None` when there's no file there.
pub async fn blob(repo: &Repo, at: &Oid, path: &str) -> Read<Option<Blob>> {
    let path = path.trim_matches('/');
    let found = entry(repo, at, path).await?;
    let Some(oid) = found.and_then(|(kind, oid)| (kind == "blob").then_some(oid)) else {
        return Ok(None);
    };
    repo.prefetch(std::slice::from_ref(&oid)).await?;
    let bytes = repo.run_bytes(&["cat-file", "blob", &oid]).await?;
    Ok(Some(blob_of(path, &bytes)))
}

/// A file's contents as a page shows them: text (its start, past
/// [`TEXT_MAX`]), or nothing for a binary file (a NUL in its first 8000
/// bytes, as git judges).
fn blob_of(path: &str, bytes: &[u8]) -> Blob {
    let binary = crate::diff::is_binary(bytes);
    let truncated = !binary && bytes.len() > TEXT_MAX;
    let text = (!binary).then(|| {
        let shown = bytes.get(..TEXT_MAX).unwrap_or(bytes);
        // Cut at the last whole line.
        let shown = match (truncated, shown.iter().rposition(|b| *b == b'\n')) {
            (true, Some(end)) => shown.get(..=end).unwrap_or(shown),
            _ => shown,
        };
        String::from_utf8_lossy(shown).into_owned()
    });
    Blob {
        path: path.to_owned(),
        text,
        size: bytes.len() as u64,
        truncated,
    }
}

/// Every file's path at `at`, and whether the list was cut at
/// [`FILES_MAX`].
pub async fn files(repo: &Repo, at: &Oid) -> Read<(Vec<String>, bool)> {
    let mut files = repo.file_names(at).await?;
    let cut = files.len() > FILES_MAX;
    files.truncate(FILES_MAX);
    Ok((files, cut))
}

/// Where a history's next page starts: how many commits come before it.
pub fn history_cursor(skip: usize) -> String {
    format!("git:{skip}")
}

/// How many commits a [`history_cursor`] skips; `None` for GitHub's own.
pub fn history_skip(cursor: &str) -> Option<usize> {
    cursor.strip_prefix("git:")?.parse().ok()
}

/// `at`'s commits, newest first, touching `path` unless it's empty: a
/// page of [`HISTORY_PAGE`] after the first `skip`.
pub async fn history(repo: &Repo, at: &Oid, path: &str, skip: usize) -> Read<Results<CommitInfo>> {
    let (format, count, skip_arg) = (
        format!("--format={INFO}"),
        format!("--max-count={}", HISTORY_PAGE + 1),
        format!("--skip={skip}"),
    );
    let mut args = vec!["log", &format, &count, &skip_arg, at, "--"];
    let path = path.trim_matches('/');
    if !path.is_empty() {
        args.push(path);
    }
    let out = repo.run(&args).await?;
    let mut items: Vec<CommitInfo> = out.lines().filter_map(commit_info).collect();
    let more = items.len() > HISTORY_PAGE;
    items.truncate(HISTORY_PAGE);
    let next = more.then(|| history_cursor(skip + HISTORY_PAGE));
    Ok(Results::uncounted(items, next))
}

/// Files changed from `from` to `to` (no rename detection: it would need
/// the files' contents).
async fn changed(repo: &Repo, from: &str, to: &str) -> Read<Vec<ChangedFile>> {
    let out = repo
        .run(&[
            "diff",
            "--raw",
            "-z",
            "--no-abbrev",
            "--no-renames",
            from,
            to,
        ])
        .await?;
    Ok(crate::git::files::parse_raw(&out)?)
}

/// Lines added and removed from `from` to `to`, after fetching the files
/// it needs in one batch; a limit past [`STAT_FILES_MAX`] files.
async fn line_counts(repo: &Repo, from: &str, to: &str, files: &[ChangedFile]) -> Read<(u64, u64)> {
    if files.len() > STAT_FILES_MAX {
        return Err(ReadError::Limit(format!(
            "{} files changed, more than the {STAT_FILES_MAX} git counts lines in",
            files.len()
        )));
    }
    repo.prefetch(&crate::git::files::blobs(files)).await?;
    let out = repo
        .run(&["diff", "--numstat", "-z", "--no-renames", from, to])
        .await?;
    let (mut added, mut removed) = (0, 0);
    for line in out.split('\0') {
        let mut f = line.split('\t');
        // A binary file's counts are `-`.
        if let (Some(a), Some(r)) = (f.next(), f.next()) {
            added += a.parse::<u64>().unwrap_or(0);
            removed += r.parse::<u64>().unwrap_or(0);
        }
    }
    Ok((added, removed))
}

/// The commit `rev` names here (a full or short ID), with its message, who
/// wrote and committed it, its parents and what it changed against its
/// first parent. `None` when there's no such commit here.
pub async fn commit(repo: &Repo, rev: &str) -> Read<Option<CommitDetail>> {
    let Ok(oid) = repo.rev_parse(rev).await else { return Ok(None) };
    let format = "--format=%H%x1f%P%x1f%an%x1f%ae%x1f%at%x1f%cn%x1f%ce%x1f%ct%x1f%B";
    let out = repo.run(&["show", "-s", format, &oid]).await?;
    let bad = || GitError::Parse(format!("show {oid}"));
    let mut f = out.splitn(9, '\u{1f}');
    let mut next = || f.next().ok_or_else(bad);
    let (_, parents, author, author_email, authored) =
        (next()?, next()?, next()?, next()?, next()?);
    let (committer, committer_email, committed, message) = (next()?, next()?, next()?, next()?);
    let message = message.trim_end();
    let (headline, body) = message.split_once('\n').unwrap_or((message, ""));
    let parents: Vec<String> = parents.split_whitespace().map(str::to_owned).collect();
    let first = parents
        .first()
        .map_or(EMPTY_TREE, String::as_str)
        .to_owned();
    let files = changed(repo, &first, &oid).await?;
    let (additions, deletions) = line_counts(repo, &first, &oid, &files).await?;
    let author_is_committer = (author, author_email) == (committer, committer_email);
    Ok(Some(CommitDetail {
        oid: oid.to_string(),
        headline: headline.trim().to_owned(),
        body: body.trim().to_owned(),
        author: Person::from_git(author, author_email),
        authored_at: iso(authored),
        committer: (!author_is_committer).then(|| Person::from_git(committer, committer_email)),
        committed_at: iso(committed),
        parents: Capped::from(parents),
        additions,
        deletions,
        changed_files: Some(files.len() as u64),
        // Whether GitHub verified a signature is GitHub's to say.
        verified: None,
    }))
}

/// Which commit last changed each line of `path` at `at`, as runs of
/// lines; `None` when there's no file there. Every version of the file is
/// fetched in one batch first (at most [`BLAME_VERSIONS_MAX`]).
pub async fn blame(repo: &Repo, at: &Oid, path: &str) -> Read<Option<Blame>> {
    let path = path.trim_matches('/');
    let versions = repo
        .run(&[
            "log",
            "--format=",
            "--raw",
            "--no-abbrev",
            "--no-renames",
            at,
            "--",
            path,
        ])
        .await?;
    let mut blobs = HashSet::new();
    for line in versions.lines().filter(|l| l.starts_with(':')) {
        let mut f = line.split_whitespace().skip(2);
        blobs.extend(
            f.by_ref()
                .take(2)
                .filter(|o| *o != ZERO_OID)
                .map(str::to_owned),
        );
    }
    if blobs.is_empty() {
        return Ok(None);
    }
    if blobs.len() > BLAME_VERSIONS_MAX {
        return Err(ReadError::Limit(format!(
            "{path} has {} versions, more than the {BLAME_VERSIONS_MAX} git blames",
            blobs.len()
        )));
    }
    repo.prefetch(&Vec::from_iter(blobs)).await?;
    let out = repo.run(&["blame", "--porcelain", at, "--", path]).await?;
    Ok(Some(parse_blame(&out)))
}

/// `git blame --porcelain`'s output as runs of lines by commit, each aged
/// from 1 (the newest commit in it) to 10 (the oldest), as GitHub's.
fn parse_blame(out: &str) -> Blame {
    #[derive(Default)]
    struct Seen {
        name: String,
        email: String,
        time: String,
        summary: String,
    }
    let mut commits: HashMap<&str, Seen> = HashMap::new();
    // (line, commit) for each line of the file.
    let mut lines: Vec<(u32, &str)> = Vec::new();
    let mut current: Option<&str> = None;
    for line in out.lines() {
        if line.starts_with('\t') {
            continue;
        }
        let mut words = line.splitn(2, ' ');
        let (key, value) = (
            words.next().unwrap_or_default(),
            words.next().unwrap_or_default(),
        );
        if key.len() == 40 && key.bytes().all(|b| b.is_ascii_hexdigit()) {
            let mut nums = value.split(' ');
            if let Some(n) = nums.nth(1).and_then(|n| n.parse().ok()) {
                lines.push((n, key));
            }
            current = Some(key);
            commits.entry(key).or_default();
            continue;
        }
        let Some(seen) = current.and_then(|c| commits.get_mut(c)) else { continue };
        match key {
            "author" => seen.name = value.to_owned(),
            "author-mail" => seen.email = value.trim_matches(['<', '>']).to_owned(),
            "committer-time" => seen.time = value.to_owned(),
            "summary" => seen.summary = value.to_owned(),
            _ => {}
        }
    }
    let times: Vec<u64> = commits
        .values()
        .filter_map(|c| c.time.parse().ok())
        .collect();
    let (oldest, newest) = (
        times.iter().copied().min().unwrap_or(0),
        times.iter().copied().max().unwrap_or(0),
    );
    let age = |time: &str| {
        let t: u64 = time.parse().unwrap_or(oldest);
        let span = newest.saturating_sub(oldest).max(1);
        let from_newest = newest.saturating_sub(t);
        u8::try_from(1 + from_newest * 9 / span).unwrap_or(10)
    };
    lines.sort_by_key(|(n, _)| *n);
    let mut ranges: Vec<BlameRange> = Vec::new();
    for (n, oid) in lines {
        if let Some(last) = ranges.last_mut()
            && last.commit.oid == oid
            && last.end + 1 == n
        {
            last.end = n;
            continue;
        }
        let Some(c) = commits.get(oid) else { continue };
        ranges.push(BlameRange {
            start: n,
            end: n,
            age: age(&c.time),
            commit: CommitInfo {
                oid: oid.to_owned(),
                headline: c.summary.clone(),
                author: Person::from_git(&c.name, &c.email),
                date: iso(&c.time),
            },
        });
    }
    Blame { ranges }
}

/// `base` compared with `head`: the commits `head` has that `base`
/// doesn't (the newest [`crate::api::browse::COMPARE_COMMITS`]), how far
/// each is ahead of the other, and the diff's size: from their merge base,
/// or from `base` itself when `direct` (`a..b`).
pub async fn compare(repo: &Repo, base: &Oid, head: &Oid, direct: bool) -> Read<Comparison> {
    let merge_base = repo.merge_base(base, head).await?;
    let from = if direct { base.clone() } else { merge_base };
    // A limit is found before the rest is read.
    let files = changed(repo, &from, head).await?;
    let (additions, deletions) = line_counts(repo, &from, head, &files).await?;
    let counts = repo
        .run(&[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base}...{head}"),
        ])
        .await?;
    let mut counts = counts
        .split_whitespace()
        .map(|n| n.parse::<u64>().unwrap_or(0));
    let (behind, ahead) = (counts.next().unwrap_or(0), counts.next().unwrap_or(0));
    let (format, count) = (
        format!("--format={INFO}"),
        format!("--max-count={}", crate::api::browse::COMPARE_COMMITS),
    );
    let range = format!("{base}..{head}");
    let out = repo.run(&["log", &format, &count, &range]).await?;
    let mut commits: Vec<CommitInfo> = out.lines().filter_map(commit_info).collect();
    commits.reverse();
    Ok(Comparison {
        ahead,
        behind,
        commits: Capped::new(commits, ahead),
        from: from.to_string(),
        to: head.to_string(),
        files: files.len() as u64,
        files_capped: false,
        additions,
        deletions,
    })
}

/// The latest commit to change each of `names` in directory `dir` at
/// `at`, from one walk of its history, which stops once each is found or
/// `enough` says to (time's up). What it didn't reach is left out.
pub async fn last_commits(
    repo: &Repo,
    at: &Oid,
    dir: &str,
    names: &[String],
    enough: impl Fn() -> bool,
) -> Read<HashMap<String, CommitInfo>> {
    let dir = dir.trim_matches('/');
    let mut wanted: HashSet<&str> = names.iter().map(String::as_str).collect();
    let format = format!("--format=%x1e{INFO}");
    let mut cmd = repo.cmd();
    cmd.args([
        "-c",
        "core.quotePath=false",
        "log",
        "--name-only",
        "--no-renames",
    ])
    .args([format.as_str(), at, "--"]);
    if !dir.is_empty() {
        cmd.arg(dir);
    }
    cmd.stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = cmd.spawn().map_err(GitError::from)?;
    let mut lines = BufReader::new(piped(child.stdout.take(), "stdout")?).lines();
    let mut found = HashMap::new();
    let mut commit: Option<CommitInfo> = None;
    while !wanted.is_empty() && !enough() {
        let Some(line) = lines.next_line().await.map_err(GitError::from)? else { break };
        if let Some(fields) = line.strip_prefix('\u{1e}') {
            commit = commit_info(fields);
            continue;
        }
        let Some(c) = &commit else { continue };
        let inside = if dir.is_empty() {
            Some(line.as_str())
        } else {
            line.strip_prefix(dir)
                .and_then(|rest| rest.strip_prefix('/'))
        };
        let Some(name) = inside.and_then(|p| p.split('/').next()) else { continue };
        if wanted.remove(name) {
            found.insert(name.to_owned(), c.clone());
        }
    }
    // Done or out of time: the rest of the walk isn't wanted.
    let _ = child.start_kill();
    let _ = child.wait().await;
    Ok(found)
}

/// Branches and tags GitHub has, as `git ls-remote` lists them (tags at
/// the commits they name), and its default branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteRefs {
    pub branches: Vec<(String, Oid)>,
    pub tags: Vec<(String, Oid)>,
}

impl RemoteRefs {
    /// The commit a branch (first) or tag named `name` is at.
    pub fn find(&self, name: &str) -> Option<&Oid> {
        (self.branches.iter())
            .chain(&self.tags)
            .find_map(|(n, oid)| (n == name).then_some(oid))
    }

    /// The longest run of `spot`'s leading segments that names a branch
    /// or tag, when longer than one segment.
    pub fn ref_in(&self, spot: &str) -> Option<String> {
        let segments: Vec<&str> = spot.split('/').collect();
        (2..=segments.len())
            .rev()
            .filter_map(|n| segments.get(..n).map(|s| s.join("/")))
            .find(|name| self.find(name).is_some())
    }
}

/// The branches and tags on the remote, in one round trip.
pub async fn remote_refs(repo: &Repo) -> Read<RemoteRefs> {
    let out = repo
        .run(&["ls-remote", "--heads", "--tags", &repo.remote])
        .await?;
    Ok(parse_ls_remote(&out))
}

fn parse_ls_remote(out: &str) -> RemoteRefs {
    let mut refs = RemoteRefs::default();
    let mut peeled: HashMap<String, Oid> = HashMap::new();
    for line in out.lines() {
        let Some((oid, name)) = line.split_once('\t') else { continue };
        let Ok(oid) = Oid::parse(oid) else { continue };
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            refs.branches.push((branch.to_owned(), oid));
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            match tag.strip_suffix("^{}") {
                Some(tag) => {
                    peeled.insert(tag.to_owned(), oid);
                }
                None => refs.tags.push((tag.to_owned(), oid)),
            }
        }
    }
    // An annotated tag names its tag object; the commit is what's read.
    for (name, oid) in &mut refs.tags {
        if let Some(commit) = peeled.remove(name) {
            *oid = commit;
        }
    }
    refs.branches.sort_by(|a, b| a.0.cmp(&b.0));
    refs
}

/// `tags` (as [`remote_refs`] lists them) newest commit first, as GitHub
/// lists them, each with its commit's date: the commits the clone doesn't
/// have are fetched first, in one batch.
pub async fn tags(repo: &Repo, tags: &[(String, Oid)]) -> Read<Vec<TagInfo>> {
    let mut oids: Vec<String> = tags.iter().map(|(_, oid)| oid.to_string()).collect();
    oids.sort();
    oids.dedup();
    let missing = repo.missing(&oids).await?;
    if !missing.is_empty() {
        let wants: Vec<&str> = missing.iter().map(String::as_str).collect();
        repo.fetch(&wants, &|_| {}).await?;
    }
    let log = ["log", "--no-walk=unsorted", "--stdin", "--format=%H %ct"];
    let out = repo.git(&log, &[], Some(oids.join("\n") + "\n")).await?;
    let dates: HashMap<&str, u64> = (out.lines())
        .filter_map(|l| l.split_once(' '))
        .filter_map(|(oid, time)| Some((oid, time.parse().ok()?)))
        .collect();
    let mut listed: Vec<(u64, TagInfo)> = (tags.iter())
        .map(|(name, oid)| {
            let time = dates.get(&**oid).copied();
            let info = TagInfo {
                name: name.clone(),
                oid: Some(oid.to_string()),
                date: time.map(crate::ui::time::iso8601),
            };
            (time.unwrap_or(0), info)
        })
        .collect();
    listed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.name.cmp(&a.1.name)));
    Ok(listed.into_iter().map(|(_, info)| info).collect())
}

/// The remote's default branch: the name its `HEAD` points at, and its
/// commit.
pub async fn remote_head(repo: &Repo) -> Read<(String, Oid)> {
    let out = repo
        .run(&["ls-remote", "--symref", &repo.remote, "HEAD"])
        .await?;
    let name = (out.lines())
        .find_map(|l| l.strip_prefix("ref: refs/heads/")?.strip_suffix("\tHEAD"))
        .ok_or_else(|| GitError::Parse("the remote has no default branch".into()))?;
    let oid = (out.lines())
        .find_map(|l| Oid::parse(l.strip_suffix("\tHEAD")?).ok())
        .ok_or_else(|| GitError::Parse("the remote's HEAD names no commit".into()))?;
    Ok((name.to_owned(), oid))
}

/// What a code search asks git for: words every file found has (each a
/// fixed string, matched without regard to case), and paths to look in.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grep {
    pub words: Vec<String>,
    /// Pathspecs (`*.rs`, `src/`); none for everywhere.
    pub paths: Vec<String>,
}

/// The files at `at` that have every word of `grep`, by path, each with its
/// first [`GREP_LINES`] matching lines. The files the clone doesn't have
/// are fetched first, in one batch (at most [`GREP_FETCH_MAX`]).
pub async fn grep(repo: &Repo, of: &RepoId, at: &Oid, grep: &Grep) -> Read<Vec<CodeHit>> {
    if repo.partial {
        let mut args = vec!["ls-tree", "-r", "-z", at.as_ref(), "--"];
        args.extend(grep.paths.iter().map(String::as_str));
        let out = repo.run(&args).await?;
        let blobs: Vec<String> = (out.split('\0'))
            .filter_map(|line| {
                let (meta, _) = line.split_once('\t')?;
                let mut meta = meta.split(' ');
                let (mode, kind, oid) = (meta.next()?, meta.next()?, meta.next()?);
                let mode = u32::from_str_radix(mode, 8).ok()?;
                (kind == "blob" && mode != MODE_SUBMODULE).then(|| oid.to_owned())
            })
            .collect();
        // Counted before asking which are here, which takes a while for
        // so many.
        if blobs.len() > GREP_FETCH_MAX {
            return Err(ReadError::Limit(format!(
                "{} files, more than the {GREP_FETCH_MAX} git searches in ghtui's clone",
                blobs.len()
            )));
        }
        repo.prefetch(&blobs).await?;
    }
    let mut args = vec![
        "-c",
        "core.quotePath=false",
        "grep",
        "-n",
        "-I",
        "-i",
        "-F",
        "-z",
        "--all-match",
    ];
    for word in &grep.words {
        args.extend(["-e", word.as_str()]);
    }
    args.extend([at.as_ref(), "--"]);
    args.extend(grep.paths.iter().map(String::as_str));
    let out = match repo.run(&args).await {
        Ok(out) => out,
        // Nothing found.
        Err(GitError::Failed { stderr, .. }) if stderr.is_empty() => String::new(),
        Err(err) => return Err(err.into()),
    };
    let prefix = format!("{at}:");
    let mut hits: Vec<CodeHit> = Vec::new();
    for line in out.lines() {
        let mut f = line.splitn(3, '\0');
        let (Some(name), Some(n), Some(text)) = (f.next(), f.next(), f.next()) else {
            continue;
        };
        let path = name.strip_prefix(&prefix).unwrap_or(name);
        let Ok(n) = n.parse::<u32>() else { continue };
        if hits.last().is_none_or(|h| h.path != path) {
            hits.push(CodeHit {
                repo: of.clone(),
                path: path.to_owned(),
                sha: String::new(),
                lines: Vec::new(),
                rev: Some(at.to_string()),
            });
        }
        if let Some(hit) = hits.last_mut()
            && hit.lines.len() < GREP_LINES
        {
            hit.lines.push((n, text.trim_end().to_owned()));
        }
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::set_up as git;
    use std::path::Path;

    async fn write(dir: &Path, files: &[(&str, &str)], message: &str, email: &str) {
        for (path, text) in files {
            let file = dir.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        git(dir, &["add", "-A"]).await;
        let author = format!("--author=Ann <{email}>");
        git(dir, &["commit", "-q", &author, "-m", message]).await;
    }

    /// "GitHub": a repository with history on `main` and a tag, and a
    /// branch `topic` off it.
    async fn github() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        crate::git::fixture::origin(d).await;
        let me = "1+ann@users.noreply.github.com";
        write(
            d,
            &[("README.md", "# Hi\n"), ("src/lib.rs", "fn a() {}\n")],
            "First",
            "ann@example.com",
        )
        .await;
        write(
            d,
            &[("src/lib.rs", "fn a() {}\nfn needle() {}\n")],
            "Add needle\n\nWhy it's there.",
            me,
        )
        .await;
        git(d, &["tag", "-a", "v1", "-m", "One"]).await;
        write(
            d,
            &[("docs/guide.md", "Needle in the docs\n")],
            "Docs",
            "ann@example.com",
        )
        .await;
        git(d, &["switch", "-q", "-c", "topic"]).await;
        write(d, &[("src/new.rs", "new\n")], "New file", "ann@example.com").await;
        git(d, &["switch", "-q", "main"]).await;
        dir
    }

    /// A partial clone of "GitHub", as the cache keeps.
    async fn partial(github: &Path) -> (tempfile::TempDir, Repo) {
        let (cache, repo) = crate::git::fixture::cache_clone(github).await;
        assert!(repo.partial);
        (cache, repo)
    }

    async fn oid(repo: &Repo, rev: &str) -> Oid {
        repo.rev_parse(rev).await.unwrap()
    }

    /// The Code tab, a file, "Go to file" and the history read from a
    /// partial clone, fetching only the file that's read.
    #[tokio::test]
    async fn a_partial_clone_reads_trees_files_and_history() {
        let github = github().await;
        let (_cache, repo) = partial(github.path()).await;
        let refs = remote_refs(&repo).await.unwrap();
        let main = refs.find("main").unwrap().clone();
        assert_eq!(refs.find("v1"), Some(&oid(&repo, "main~1").await), "peeled");
        assert_eq!(refs.ref_in("topic/src/new.rs"), None, "one segment");
        assert_eq!(
            remote_head(&repo).await.unwrap(),
            ("main".into(), main.clone())
        );

        let root = tree(&repo, &main, "").await.unwrap().unwrap();
        let names: Vec<_> = root.iter().map(|e| (e.name.as_str(), e.kind)).collect();
        assert_eq!(
            names,
            [
                ("docs", EntryKind::Dir),
                ("src", EntryKind::Dir),
                ("README.md", EntryKind::File)
            ]
        );
        let src = tree(&repo, &main, "src").await.unwrap().unwrap();
        assert_eq!(src[0].path, "src/lib.rs");
        assert_eq!(tree(&repo, &main, "README.md").await.unwrap(), None);

        let readme = repo
            .run(&["rev-parse", &format!("{main}:README.md")])
            .await
            .unwrap();
        assert!(!repo.has(readme.trim()).await, "blobless");
        let file = blob(&repo, &main, "README.md").await.unwrap().unwrap();
        assert_eq!(file.text.as_deref(), Some("# Hi\n"));
        assert!(repo.has(readme.trim()).await, "fetched to read it");
        assert_eq!(blob(&repo, &main, "src").await.unwrap(), None);

        let (paths, cut) = files(&repo, &main).await.unwrap();
        assert_eq!((paths.len(), cut), (3, false));

        let all = history(&repo, &main, "", 0).await.unwrap();
        let heads: Vec<_> = all.items.iter().map(|c| c.headline.as_str()).collect();
        assert_eq!(heads, ["Docs", "Add needle", "First"]);
        assert_eq!(all.next, None);
        let lib = history(&repo, &main, "src/lib.rs", 0).await.unwrap();
        assert_eq!(lib.items.len(), 2);
        assert_eq!(
            lib.items[0].author.login(),
            Some("ann"),
            "a no-reply email is its account"
        );
        assert_eq!(lib.items[1].author, Person::Git("Ann".into()));
        assert_eq!(history_skip(&history_cursor(30)), Some(30));

        let last = last_commits(
            &repo,
            &main,
            "",
            &["src".into(), "README.md".into()],
            || false,
        )
        .await
        .unwrap();
        assert_eq!(last["src"].headline, "Add needle");
        assert_eq!(last["README.md"].headline, "First");
    }

    /// A commit, blame and a comparison count lines, after fetching what
    /// they read in one batch.
    #[tokio::test]
    async fn commits_blame_and_comparisons_read_from_git() {
        let github = github().await;
        let (_cache, repo) = partial(github.path()).await;
        let main = oid(&repo, "main").await;

        let c = commit(&repo, "main~1").await.unwrap().unwrap();
        assert_eq!(
            (c.headline.as_str(), c.body.as_str()),
            ("Add needle", "Why it's there.")
        );
        assert_eq!((c.additions, c.deletions, c.changed_files), (1, 0, Some(1)));
        assert_eq!(c.parents.len(), 1);
        assert_eq!(c.author.login(), Some("ann"));
        assert!(
            c.committer.is_some(),
            "Ann committed it under another email"
        );
        let short = crate::ui::text::short_sha(&c.oid);
        assert_eq!(commit(&repo, short).await.unwrap().unwrap().oid, c.oid);
        let root = commit(&repo, "main~2").await.unwrap().unwrap();
        assert_eq!((root.additions, root.changed_files), (2, Some(2)));

        let b = blame(&repo, &main, "src/lib.rs").await.unwrap().unwrap();
        let runs: Vec<_> = (b.ranges.iter())
            .map(|r| (r.start, r.end, r.commit.headline.as_str()))
            .collect();
        assert_eq!(runs, [(1, 1, "First"), (2, 2, "Add needle")]);
        // The newer commit is no older than the first, however far apart
        // a slow machine made them.
        let ages: Vec<_> = b.ranges.iter().map(|r| r.age).collect();
        assert!(ages.get(1) <= ages.first(), "{ages:?}");
        assert_eq!(blame(&repo, &main, "nothing.rs").await.unwrap(), None);

        let topic = repo.rev_parse("refs/heads/topic").await;
        // The bare clone has GitHub's branches as they were cloned.
        let topic = topic.unwrap();
        let cmp = compare(&repo, &main, &topic, false).await.unwrap();
        assert_eq!((cmp.ahead, cmp.behind, cmp.files), (1, 0, 1));
        assert_eq!((cmp.additions, cmp.deletions), (1, 0));
        assert_eq!(cmp.commits.items[0].headline, "New file");
        let back = compare(&repo, &topic, &oid(&repo, "main~2").await, true)
            .await
            .unwrap();
        assert_eq!((back.ahead, back.behind), (0, 3));
    }

    /// Code search finds the files with every word, case aside, with their
    /// lines, fetching the files first.
    #[tokio::test]
    async fn grep_finds_files_with_every_word() {
        let github = github().await;
        let (_cache, repo) = partial(github.path()).await;
        let main = oid(&repo, "main").await;
        let of = RepoId::parse("o/r").unwrap();
        let ask = |words: &[&str], paths: &[&str]| Grep {
            words: words.iter().map(|w| (*w).to_owned()).collect(),
            paths: paths.iter().map(|p| (*p).to_owned()).collect(),
        };
        let hits = grep(&repo, &of, &main, &ask(&["needle"], &[]))
            .await
            .unwrap();
        let found: Vec<_> = hits
            .iter()
            .map(|h| (h.path.as_str(), h.lines.clone()))
            .collect();
        assert_eq!(
            found,
            [
                ("docs/guide.md", vec![(1, "Needle in the docs".to_owned())]),
                ("src/lib.rs", vec![(2, "fn needle() {}".to_owned())]),
            ]
        );
        let both = grep(&repo, &of, &main, &ask(&["needle", "fn a"], &[]))
            .await
            .unwrap();
        assert_eq!(both.len(), 1);
        let rs = grep(&repo, &of, &main, &ask(&["needle"], &["*.md"]))
            .await
            .unwrap();
        assert_eq!(rs[0].path, "docs/guide.md");
        assert!(
            grep(&repo, &of, &main, &ask(&["nowhere"], &[]))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_big_file_shows_its_start_and_a_binary_one_nothing() {
        let big = "line\n".repeat(TEXT_MAX / 4);
        let b = blob_of("big.txt", big.as_bytes());
        assert!(b.truncated);
        assert!(b.text.as_ref().unwrap().len() <= TEXT_MAX);
        assert!(b.text.unwrap().ends_with('\n'));
        let bin = blob_of("x.bin", b"PNG\0\x01");
        assert_eq!((bin.text, bin.size), (None, 5));
    }
}
