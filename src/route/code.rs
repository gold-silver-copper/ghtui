//! A repository's code: its files at a revision, their history and
//! blame, commits and comparisons, and its branches and tags.

use super::{Family, Route, Target, Url, valid_rev};
use crate::api::browse::CommitDetail;
use crate::api::model::{PrRef, RepoId};
use crate::browse::{Build, Width};
use crate::chrome::{Chrome, compare_tabs, repo_crumbs};
use crate::data::{Data, DataKey};
use crate::diff_screen::DiffOf;
use crate::git::Oid;
use crate::route::{pulls, runs};
use crate::state::State;
use crate::ui::pages::{self, url as links};
use crate::ui::text::short_sha;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    /// The Code tab at the repository root.
    Repo(RepoId),
    Tree {
        repo: RepoId,
        rev: String,
        path: String,
    },
    Blob {
        repo: RepoId,
        rev: String,
        path: String,
        /// Lines a link points at (`#L10-L20`), marked and scrolled to.
        lines: Option<(u32, u32)>,
    },
    /// Which commit last changed each line of a file.
    Blame {
        repo: RepoId,
        rev: String,
        path: String,
        lines: Option<(u32, u32)>,
    },
    /// A page at a revision from a plain GitHub URL, which doesn't say
    /// where the ref ends and the path starts (`tree/feature/x/src`).
    /// GitHub takes the longest leading part naming a branch or tag; ghtui
    /// finds it, then shows the page (see [`Page::split`]).
    Unsplit {
        repo: RepoId,
        view: RevView,
        /// The ref and the path, as the URL has them.
        spot: String,
        lines: Option<(u32, u32)>,
    },
    /// A revision's commits, of `path` if it isn't empty.
    Commits {
        repo: RepoId,
        rev: String,
        path: String,
    },
    /// A commit, by SHA (short or full, as linked); opened from a pull
    /// request's commits, that pull request's number.
    Commit {
        repo: RepoId,
        oid: String,
        pr: Option<u64>,
    },
    /// The checks on a commit.
    CommitChecks {
        repo: RepoId,
        oid: String,
    },
    /// Two revisions compared, as the URL names them (`a...b`, `a..b`).
    Compare {
        repo: RepoId,
        spec: String,
    },
    Branches(RepoId),
    Tags(RepoId),
}

pub use Page::*;

/// The pages at a revision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RevView {
    Tree,
    Blob,
    Blame,
    Commits,
}

impl RevView {
    const ALL: [RevView; 4] = [
        RevView::Tree,
        RevView::Blob,
        RevView::Blame,
        RevView::Commits,
    ];

    fn name(self) -> &'static str {
        match self {
            RevView::Tree => "tree",
            RevView::Blob => "blob",
            RevView::Blame => "blame",
            RevView::Commits => "commits",
        }
    }

    /// The page at `rev` (blame needs a file).
    fn at(
        self,
        repo: RepoId,
        rev: String,
        path: String,
        lines: Option<(u32, u32)>,
    ) -> Option<Page> {
        Some(match self {
            RevView::Tree => Tree { repo, rev, path },
            RevView::Blob => Blob {
                repo,
                rev,
                path,
                lines,
            },
            RevView::Blame if path.is_empty() => return None,
            RevView::Blame => Blame {
                repo,
                rev,
                path,
                lines,
            },
            RevView::Commits => Commits { repo, rev, path },
        })
    }
}

impl Page {
    /// The revision the page is at.
    pub fn rev(&self) -> Option<&str> {
        match self {
            Tree { rev, .. } | Blob { rev, .. } | Blame { rev, .. } | Commits { rev, .. } => {
                Some(rev)
            }
            _ => None,
        }
    }

    /// An [`Page::Unsplit`] page once its ref is known: `rev` if the spot
    /// starts with it, else the first segment.
    pub fn split(&self, rev: Option<&str>) -> Option<Route> {
        let Unsplit {
            repo,
            view,
            spot,
            lines,
        } = self
        else {
            return None;
        };
        let after = |rev: &str| {
            let rest = spot.strip_prefix(rev)?;
            match rest.strip_prefix('/') {
                Some(path) => Some(path.to_owned()),
                None => rest.is_empty().then(String::new),
            }
        };
        let at = |rev: &str| view.at(repo.clone(), rev.to_owned(), after(rev)?, *lines);
        let first = spot.split('/').next().unwrap_or_default();
        rev.and_then(at).or_else(|| at(first)).map(Route::Code)
    }

    fn header(&self) -> DataKey {
        DataKey::Repo(self.repo_id().clone())
    }

    fn repo_id(&self) -> &RepoId {
        match self {
            Repo(repo)
            | Tree { repo, .. }
            | Blob { repo, .. }
            | Blame { repo, .. }
            | Unsplit { repo, .. }
            | Commits { repo, .. }
            | Commit { repo, .. }
            | CommitChecks { repo, .. }
            | Compare { repo, .. }
            | Branches(repo)
            | Tags(repo) => repo,
        }
    }
}

impl Family for Page {
    fn url(&self) -> String {
        match self {
            Repo(repo) => links::repo(repo),
            Tree { repo, rev, path } => links::tree(repo, rev, path),
            Blob {
                repo,
                rev,
                path,
                lines,
            } => lined(links::blob(repo, rev, path), *lines),
            Blame {
                repo,
                rev,
                path,
                lines,
            } => lined(links::blame(repo, rev, path), *lines),
            Unsplit {
                repo,
                view,
                spot,
                lines,
            } => lined(
                format!(
                    "{}/{}/{}",
                    links::repo(repo),
                    view.name(),
                    links::encode_path(spot)
                ),
                *lines,
            ),
            Commits { repo, rev, path } => links::commits(repo, rev, path),
            Commit {
                repo,
                oid,
                pr: Some(number),
            } => {
                let pr = PrRef {
                    repo: repo.clone(),
                    number: *number,
                };
                links::pull_tab(&pr, &format!("commits/{oid}"))
            }
            Commit { repo, oid, .. } => links::commit(repo, oid),
            CommitChecks { repo, oid } => format!("{}/checks", links::commit(repo, oid)),
            Compare { repo, spec } => {
                format!("{}/compare/{}", links::repo(repo), links::encode_path(spec))
            }
            Branches(repo) => format!("{}/branches", links::repo(repo)),
            Tags(repo) => format!("{}/tags", links::repo(repo)),
        }
    }

    fn title(&self) -> String {
        match self {
            Repo(repo) => repo.to_string(),
            Tree { repo, path, .. } if path.is_empty() => repo.to_string(),
            Tree { repo, path, .. } | Blob { repo, path, .. } => format!("{}/{path}", repo.name),
            Blame { repo, path, .. } => format!("{}/{path} · Blame", repo.name),
            // Most refs are one segment.
            Unsplit { repo, spot, .. } => match self.split(None) {
                Some(page) => page.title(),
                None => format!("{}/{spot}", repo.name),
            },
            Commits { repo, path, .. } if path.is_empty() => format!("{repo} · Commits"),
            Commits { repo, path, .. } => format!("{}/{path} · Commits", repo.name),
            Commit { repo, oid, .. } => format!("{repo}@{}", short_sha(oid)),
            CommitChecks { repo, oid } => format!("{repo}@{} · Checks", short_sha(oid)),
            Compare { repo, spec } => format!("{repo} · {spec}"),
            Branches(repo) => format!("{repo} · Branches"),
            Tags(repo) => format!("{repo} · Tags"),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        Some(self.repo_id())
    }

    fn up(&self, state: &State) -> Option<Route> {
        let default = |repo: &RepoId| state.overview(repo).and_then(|o| o.default_branch.clone());
        let folder = |repo: &RepoId, rev: &str, path: &str| {
            let parent = path.rsplit_once('/').map_or("", |(p, _)| p);
            if parent.is_empty() && default(repo).as_deref() == Some(rev) {
                Repo(repo.clone())
            } else {
                Tree {
                    repo: repo.clone(),
                    rev: rev.to_owned(),
                    path: parent.to_owned(),
                }
            }
        };
        Some(Route::Code(match self {
            Repo(repo) => return Some(Route::user(&repo.owner)),
            Tree { repo, path, .. } if path.is_empty() => Repo(repo.clone()),
            Tree { repo, rev, path }
            | Blob {
                repo, rev, path, ..
            } => folder(repo, rev, path),
            Blame {
                repo, rev, path, ..
            } => return Some(Route::blob(repo.clone(), rev.clone(), path.clone())),
            Unsplit { repo, .. }
            | Commits { repo, .. }
            | Compare { repo, .. }
            | Branches(repo)
            | Tags(repo) => Repo(repo.clone()),
            CommitChecks { repo, oid } => Commit {
                repo: repo.clone(),
                oid: oid.clone(),
                pr: None,
            },
            // A pull request's commit leads up to the pull request.
            Commit {
                repo,
                pr: Some(number),
                ..
            } => {
                return Some(Route::pr(PrRef {
                    repo: repo.clone(),
                    number: *number,
                }));
            }
            // A commit's history leads up to it.
            Commit { repo, oid, .. } => Commits {
                repo: repo.clone(),
                rev: oid.clone(),
                path: String::new(),
            },
        }))
    }

    fn needs(&self) -> Vec<DataKey> {
        use DataKey as K;
        let own = match self {
            Repo(repo) => vec![
                K::Readme(repo.clone()),
                K::LastCommits(repo.clone(), "HEAD".into(), String::new()),
            ],
            Tree { repo, rev, path } => vec![
                K::Tree(repo.clone(), rev.clone(), path.clone()),
                K::LastCommits(repo.clone(), rev.clone(), path.clone()),
            ],
            Blob {
                repo, rev, path, ..
            } => vec![
                K::Codeowners(repo.clone(), rev.clone()),
                K::Blob(repo.clone(), rev.clone(), path.clone()),
            ],
            Blame {
                repo, rev, path, ..
            } => vec![
                K::Blob(repo.clone(), rev.clone(), path.clone()),
                K::Blame(repo.clone(), rev.clone(), path.clone()),
            ],
            Unsplit { repo, spot, .. } => vec![K::RefIn(repo.clone(), spot.clone())],
            Commit { repo, oid, pr } => {
                let mut needs = vec![
                    K::CommitComments(repo.clone(), oid.clone()),
                    K::Commit(repo.clone(), oid.clone()),
                ];
                // Its place among the pull request's commits.
                if let Some(number) = pr {
                    let pr = PrRef {
                        repo: repo.clone(),
                        number: *number,
                    };
                    needs.push(K::PrActivity(pr));
                }
                needs
            }
            CommitChecks { repo, oid } => vec![
                K::Commit(repo.clone(), oid.clone()),
                K::CommitChecks(repo.clone(), oid.clone()),
            ],
            // Whether ghtui runs in a clone says how one is made or
            // deleted.
            Branches(repo) | Tags(repo) => std::iter::once(K::Clone(repo.clone()))
                .chain(self.paged())
                .collect(),
            Commits { .. } | Compare { .. } => self.paged().into_iter().collect(),
        };
        std::iter::once(self.header()).chain(own).collect()
    }

    fn paged(&self) -> Option<DataKey> {
        Some(match self {
            Commits { repo, rev, path } => {
                DataKey::History(repo.clone(), rev.clone(), path.clone())
            }
            Compare { repo, spec } => DataKey::Compare(repo.clone(), spec.clone()),
            Branches(repo) => DataKey::Branches(repo.clone()),
            Tags(repo) => DataKey::Tags(repo.clone()),
            _ => return None,
        })
    }

    fn width(&self) -> Width {
        match self {
            Repo(_) => Width::ListsAside,
            Blob { path, .. } if pages::is_markdown(path) => Width::Reading,
            Blob { .. } | Blame { .. } | CommitChecks { .. } | Branches(_) | Tags(_) => Width::Full,
            Tree { .. } | Unsplit { .. } | Commits { .. } | Commit { .. } | Compare { .. } => {
                Width::Lists
            }
        }
    }

    fn starts_on_row(&self) -> bool {
        !matches!(
            self,
            Blob { lines: None, .. } | Blame { lines: None, .. } | Commit { .. }
        )
    }

    fn shown_source(&self) -> Option<DataKey> {
        Some(match self {
            Tree { repo, rev, path } => DataKey::Tree(repo.clone(), rev.clone(), path.clone()),
            Blob {
                repo, rev, path, ..
            } => DataKey::Blob(repo.clone(), rev.clone(), path.clone()),
            Blame {
                repo, rev, path, ..
            } => DataKey::Blame(repo.clone(), rev.clone(), path.clone()),
            Commit { repo, oid, .. } => DataKey::Commit(repo.clone(), oid.clone()),
            Commits { .. } | Compare { .. } | Tags(_) => return self.paged(),
            Repo(_) | Unsplit { .. } | CommitChecks { .. } | Branches(_) => return None,
        })
    }

    fn chrome(&self, state: &State, c: &mut Chrome) {
        match self {
            Commit { repo, oid, pr } => {
                repo_crumbs(repo, c);
                state.commit_tabs(repo, oid, *pr, c);
                c.active = Some(0);
            }
            CommitChecks { repo, oid } => {
                repo_crumbs(repo, c);
                state.commit_tabs(repo, oid, None, c);
                c.active = Some(1);
            }
            // Its commits, and its files once known.
            Compare { repo, spec } => {
                repo_crumbs(repo, c);
                let key = DataKey::Compare(repo.clone(), spec.clone());
                let comparison = match state.get(&key) {
                    Some(Data::Compare(cmp)) => Some(&**cmp),
                    _ => None,
                };
                let range = comparison
                    .and_then(|cmp| Some((Oid::parse(&cmp.from).ok()?, Oid::parse(&cmp.to).ok()?)));
                compare_tabs(repo, spec, range, comparison, c);
                c.active = Some(0);
            }
            _ => state.repo_chrome(self.repo_id(), self.section(), c),
        }
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let (cx, now) = (b.cx, b.cx.now);
        let title = self.title();
        match self {
            Repo(repo) => {
                let overview = b.get(&self.header());
                #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
                pages::repo_title(page, repo, overview.ready_unchecked());
                if let Some(o) = overview.show(page, "the repository") {
                    let key = DataKey::LastCommits(repo.clone(), "HEAD".into(), String::new());
                    #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
                    let commits = b.get(&key).ready_unchecked();
                    let readme = b.get(&DataKey::Readme(repo.clone()));
                    pages::repo_code(page, repo, o, commits, readme, cx);
                }
            }
            Tree { repo, rev, path } => {
                let key = DataKey::LastCommits(repo.clone(), rev.clone(), path.clone());
                #[expect(clippy::disallowed_methods, reason = "extra: shown once loaded")]
                let commits = b.get(&key).ready_unchecked();
                let dir = pages::Listing {
                    repo,
                    rev,
                    path,
                    entries: b.get(&DataKey::Tree(repo.clone(), rev.clone(), path.clone())),
                    commits,
                };
                pages::repo_dir(page, dir, cx);
            }
            Blob {
                repo,
                rev,
                path,
                lines,
            } => {
                let key = DataKey::Blob(repo.clone(), rev.clone(), path.clone());
                if let Some(blob) = b.get(&key).show(page, "the file") {
                    let file = pages::FileAt {
                        repo,
                        rev,
                        path,
                        lines: *lines,
                    };
                    let owners = b.get(&DataKey::Codeowners(repo.clone(), rev.clone()));
                    pages::file(page, file, blob, owners, cx.keys);
                }
            }
            Blame {
                repo,
                rev,
                path,
                lines,
            } => {
                let blob = b.get(&DataKey::Blob(repo.clone(), rev.clone(), path.clone()));
                let blame = b.get(&DataKey::Blame(repo.clone(), rev.clone(), path.clone()));
                let file = pages::FileAt {
                    repo,
                    rev,
                    path,
                    lines: *lines,
                };
                pages::blame(page, file, blob, blame, cx.keys, now);
            }
            // Shown until the ref is known, then replaced (`State::settle`).
            Unsplit { repo, spot, .. } => {
                let key = DataKey::RefIn(repo.clone(), spot.clone());
                let _ = b.get::<Option<String>>(&key).show(page, "the branch");
            }
            Commits { repo, rev, path } => {
                pages::commit_history(page, repo, rev, path, b.list(self.paged()), now);
            }
            Commit { repo, oid, pr } => {
                let key = DataKey::Commit(repo.clone(), oid.clone());
                if let Some(c) = b.get::<CommitDetail>(&key).show(page, &title) {
                    if let Some(at) = pr.and_then(|n| b.state.pr_commit_at(repo, n, &c.oid)) {
                        pages::pr_commit_nav(page, &at);
                    }
                    let files = Commit {
                        repo: repo.clone(),
                        oid: c.oid.clone(),
                        pr: None,
                    };
                    let files = format!("{}#files", files.url());
                    pages::commit(page, repo, c, &files, now);
                    let comments = DataKey::CommitComments(repo.clone(), oid.clone());
                    pages::commit_comments(page, repo, c, b.get(&comments), cx);
                }
            }
            CommitChecks { repo, oid } => {
                let checks = b.get(&DataKey::CommitChecks(repo.clone(), oid.clone()));
                pages::checks(page, checks, now);
            }
            Compare { repo, spec } => {
                if let Some(c) = b.list(self.paged()).show(page, &title) {
                    pages::compare(page, repo, spec, c, now);
                }
            }
            Branches(repo) => pages::branches(page, repo, b.list(self.paged()), cx.icons, now),
            Tags(repo) => pages::tags(page, repo, b.list(self.paged()), now),
        }
    }
}

/// `url` at a file's `lines`, if any.
fn lined(url: String, lines: Option<(u32, u32)>) -> String {
    match lines {
        None => url,
        Some((a, b)) if a == b => format!("{url}#L{a}"),
        Some((a, b)) => format!("{url}#L{a}-L{b}"),
    }
}

/// The repository's code pages, from what follows `/<owner>/<repo>` in
/// their URLs.
pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], url: &Url<'_>) -> Option<Target> {
    let repo = repo.clone();
    let fragment = url.fragment();
    // A link to its files: `#files`, or a file's `#diff-…`.
    let files = fragment.is_some_and(|f| f == "files" || f.starts_with("diff-"));
    let page = match rest {
        [] => Repo(repo),
        [view @ ("tree" | "blob" | "blame" | "commits"), rest @ ..] => {
            rev_page(repo, view, rest, fragment)?
        }
        ["tags"] => Tags(repo),
        // All, active, stale, yours: one list.
        ["branches", ..] => Branches(repo),
        // A comparison's patch and diff are downloads.
        ["compare", .., last] if last.ends_with(".patch") || last.ends_with(".diff") => {
            return None;
        }
        ["compare", rest @ ..] => {
            let spec = rest.join("/");
            // Without revisions it's the form for opening a pull request.
            if spec.is_empty() {
                return Some(Target::Page(Route::Pulls(pulls::NewPr(repo))));
            }
            // Its files, from one commit to the other (`a..b`; with three
            // dots it's from their merge base, which the comparison page
            // finds).
            if files
                && !spec.contains("...")
                && let Some((from, to)) = spec.split_once("..")
                && let Ok(from) = Oid::parse(from)
                && let Ok(to) = Oid::parse(to)
            {
                return Some(Target::Files(DiffOf::Range(repo, from, to)));
            }
            Compare { repo, spec }
        }
        // A commit is named by its SHA or a ref, which git keeps to
        // printable characters without spaces.
        ["commit", rev, ..] | ["git", "commit", rev] if !valid_rev(rev) => return None,
        // A check run's logs, from a pull request's checks.
        ["commit", _, "checks", job, ..] => {
            return Some(Target::Page(Route::Runs(runs::Job {
                repo,
                run: None,
                job: job.parse().ok()?,
                step: None,
                query: String::new(),
            })));
        }
        ["commit", sha, "checks"] => CommitChecks {
            repo,
            oid: (*sha).to_owned(),
        },
        ["git", "commit", sha] => Commit {
            repo,
            oid: (*sha).to_owned(),
            pr: None,
        },
        // A commit's patch is a download; its diff is the diff viewer.
        ["commit", sha] if sha.ends_with(".patch") => return None,
        // GitHub anchors a commit's files as `#diff-…`. A short SHA or a
        // ref names no commit until resolved: its page.
        ["commit", rev] => {
            let (rev, diff) = rev
                .strip_suffix(".diff")
                .map_or((*rev, false), |r| (r, true));
            if (diff || files)
                && let Ok(oid) = Oid::parse(rev)
            {
                return Some(Target::Files(DiffOf::Commit(repo, oid)));
            }
            Commit {
                repo,
                oid: rev.to_owned(),
                pr: None,
            }
        }
        _ => return None,
    };
    Some(Target::Page(Route::Code(page)))
}

/// A page at a revision, from what follows its view. Where the ref ends
/// is plain when it's the only segment, when its '/' came escaped (as
/// ghtui writes it), or when it's `HEAD` or a full SHA; otherwise it's for
/// GitHub to say ([`Page::Unsplit`]).
fn rev_page(repo: RepoId, view: &str, rest: &[&str], fragment: Option<&str>) -> Option<Page> {
    let view = RevView::ALL.into_iter().find(|v| v.name() == view)?;
    let lines = fragment.and_then(line_range);
    match rest {
        [] if view == RevView::Commits => view.at(repo, "HEAD".into(), String::new(), lines),
        [rev, ..] if !valid_rev(rev) => None,
        [rev, path @ ..]
            if path.is_empty()
                || rev.contains('/')
                || *rev == "HEAD"
                || (rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit())) =>
        {
            view.at(repo, (*rev).to_owned(), path.join("/"), lines)
        }
        [_, ..] => Some(Unsplit {
            repo,
            view,
            spot: rest.join("/"),
            lines,
        }),
        [] => None,
    }
}

/// `L10` or `L10-L20` (as GitHub writes line links), in order.
fn line_range(fragment: &str) -> Option<(u32, u32)> {
    let line = |s: &str| s.strip_prefix('L')?.parse::<u32>().ok().filter(|&n| n > 0);
    let (a, b) = match fragment.split_once('-') {
        Some((a, b)) => (line(a)?, line(b)?),
        None => (line(fragment)?, line(fragment)?),
    };
    Some((a.min(b), a.max(b)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::{path, repo, rev};
    use proptest::prelude::*;
    let lines = || {
        prop::option::of((1..1000u32, 1..1000u32))
            .prop_map(|l| l.map(|(a, b)| (a.min(b), a.max(b))))
    };
    let page = prop_oneof![
        repo().prop_map(Repo),
        (repo(), rev(), path()).prop_map(|(repo, rev, path)| Tree { repo, rev, path }),
        (repo(), rev(), path(), lines())
            .prop_filter("a file has a path", |(_, _, p, _)| !p.is_empty())
            .prop_map(|(repo, rev, path, lines)| Blob {
                repo,
                rev,
                path,
                lines,
            }),
        (repo(), rev(), path(), lines())
            .prop_filter("a file has a path", |(_, _, p, _)| !p.is_empty())
            .prop_map(|(repo, rev, path, lines)| Blame {
                repo,
                rev,
                path,
                lines,
            }),
        (repo(), rev(), path()).prop_map(|(repo, rev, path)| Commits { repo, rev, path }),
        (repo(), "[0-9a-f]{7,40}").prop_map(|(repo, oid)| Commit {
            repo,
            oid,
            pr: None
        }),
        (repo(), "[0-9a-f]{40}").prop_map(|(repo, oid)| CommitChecks { repo, oid }),
        (repo(), rev(), rev(), any::<bool>()).prop_map(|(repo, a, b, two)| Compare {
            repo,
            spec: format!("{a}{}{b}", if two { ".." } else { "..." }),
        }),
        repo().prop_map(Branches),
        repo().prop_map(Tags),
    ];
    page.prop_map(Route::Code)
}
