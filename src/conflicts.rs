//! Resolving a pull request's conflicts with its base, git first, where
//! the website has its conflict editor: in the clone the diff is in
//! (yours or the cache's), git merges GitHub's tip of the base into
//! GitHub's tip of the head, in objects alone ([`crate::git::merge`]).
//! The diff viewer shows each conflicted file against the head's, its
//! conflicts between markers; each is resolved with your side (`<`), the
//! base's (`>`) or an edit in `$EDITOR` (`e`), and the diff shows the
//! result as it goes. Committing makes the merge commit GitHub's editor
//! makes, after asking, and pushes it to the head branch; stopping at
//! any step leaves GitHub and the clone's branches as they were.

use std::sync::Arc;

use crate::act::Subject;
use crate::api::model::{Mergeable, PrRef, RepoId};
use crate::diff::FileDiff;
use crate::diff::anchor::Side;
use crate::diff_screen::{self, DiffOf, Pane};
use crate::follow::{Expectation, Shown};
use crate::forms::Answer;
use crate::git::branch::{self, refused};
use crate::git::files::{ChangedFile, FileStatus, ZERO_OID};
use crate::git::merge::{self, Content, Entry, Part};
use crate::git::{Oid, commit};
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::review::EditPurpose;
use crate::route::Route;
use crate::state::{Cmd, Cmds, Screen, State};
use crate::ui::text::{noun, short_sha};

/// `n` and its noun: "2 conflicts".
fn count(n: usize, word: &str) -> String {
    format!("{n} {}", noun(n as u64, word))
}
use crate::work::{Cx, Failure, Job};

/// What resolving conflicts adds: starting on a conflicting pull request,
/// and on its conflicts' diff, each one's resolution, committing and
/// stopping.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[
        Action::ResolveConflicts,
        Action::TakeOurs,
        Action::TakeTheirs,
        Action::CommitMerge,
        Action::AbortMerge,
    ],
    takes: |state, action| action == Action::EditComment && on_conflicts(state).is_some(),
    act,
    offers,
    ..crate::features::Feature::NONE
};

/// A merge being resolved: what was merged into what, what git made of
/// it, and each conflicted file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Merging {
    pub pr: PrRef,
    pub base_ref: String,
    pub head_ref: String,
    /// Where the head branch is.
    pub head_repo: RepoId,
    /// GitHub's tips of the head and the base, merged.
    pub head: Oid,
    pub base: Oid,
    /// What git merged them to, conflicts and all.
    pub tree: Oid,
    /// The commit the diff shows: the head with each conflicted file as
    /// it's being resolved.
    pub view: Oid,
    pub files: Vec<File>,
}

/// A conflicted file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct File {
    pub path: String,
    /// The head's file, the diff's old side; none where it has none.
    pub ours: Option<Vec<u8>>,
    pub how: How,
}

/// How a file conflicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum How {
    /// In places of its text, between markers.
    Hunks(Vec<Part>),
    /// Whole: changed on one side and deleted on the other, or not text.
    /// Each side as it has it, if it does, and the one chosen.
    Whole {
        ours: Option<(Entry, Vec<u8>)>,
        theirs: Option<(Entry, Vec<u8>)>,
        chosen: Option<Pick>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pick {
    Ours,
    Theirs,
}

impl Merging {
    pub fn of(&self) -> DiffOf {
        DiffOf::Conflicts(self.pr.clone(), self.head.clone(), self.view.clone())
    }

    /// Conflicts still to resolve.
    fn left(&self) -> usize {
        self.files.iter().map(File::left).sum()
    }

    /// The commit's message, as GitHub's conflict editor writes it.
    fn message(&self) -> String {
        format!("Merge branch '{}' into {}", self.base_ref, self.head_ref)
    }
}

impl File {
    fn left(&self) -> usize {
        match &self.how {
            How::Hunks(parts) => (parts.iter())
                .filter(|p| matches!(p, Part::Conflict { chosen: None, .. }))
                .count(),
            How::Whole { chosen, .. } => usize::from(chosen.is_none()),
        }
    }

    /// Its text as it's being resolved, between markers labelled with the
    /// branches; none where it's gone.
    fn now(&self, m: &Merging) -> Option<Vec<u8>> {
        match &self.how {
            How::Hunks(parts) => Some(merge::render(parts, &m.head_ref, &m.base_ref).into_bytes()),
            How::Whole {
                ours,
                theirs,
                chosen,
            } => {
                let side = match chosen {
                    Some(Pick::Ours) => ours,
                    Some(Pick::Theirs) | None => theirs,
                };
                side.as_ref().map(|(_, bytes)| bytes.clone())
            }
        }
    }

    /// Its diff from the head's, as it's being resolved.
    fn diff(&self, m: &Merging) -> FileDiff {
        let now = self.now(m);
        FileDiff::compute(&self.path, self.ours.as_deref(), now.as_deref())
    }

    /// Its change from the head's as it's being resolved, the diff's
    /// listing of it (`was`) brought up to date: a side taken whole may
    /// keep what the other deleted, or delete it.
    fn change(&self, was: &ChangedFile) -> ChangedFile {
        let How::Whole {
            ours,
            theirs,
            chosen: Some(pick),
        } = &self.how
        else {
            return was.clone();
        };
        let side = match pick {
            Pick::Ours => ours,
            Pick::Theirs => theirs,
        };
        let mut now = was.clone();
        let had = self.ours.is_some();
        let path = Some(self.path.clone());
        (now.old_path, now.new_path) = (path.clone().filter(|_| had), None);
        match side {
            Some((entry, _)) => {
                now.new_path = path;
                now.new_oid = entry.oid.clone();
                now.new_mode = u32::from_str_radix(&entry.mode, 8).unwrap_or(0o100644);
                now.status = if had {
                    FileStatus::Modified
                } else {
                    FileStatus::Added
                };
            }
            None => {
                now.new_oid = Oid::parse(ZERO_OID).unwrap_or_else(|_| was.new_oid.clone());
                now.new_mode = 0;
                now.status = FileStatus::Deleted;
            }
        }
        now
    }

    /// What it holds in the merge commit, once it's resolved.
    fn content(&self, m: &Merging) -> Option<Content> {
        match &self.how {
            How::Hunks(parts) => {
                let resolved = parts
                    .iter()
                    .all(|p| !matches!(p, Part::Conflict { chosen: None, .. }));
                resolved.then(|| Content::Text(merge::render(parts, &m.head_ref, &m.base_ref)))
            }
            How::Whole {
                ours,
                theirs,
                chosen,
            } => {
                let side = match (*chosen)? {
                    Pick::Ours => ours,
                    Pick::Theirs => theirs,
                };
                Some(
                    side.as_ref()
                        .map_or(Content::Gone, |(e, _)| Content::Entry(e.clone())),
                )
            }
        }
    }

    /// The lines of its text each conflict is on (first, last), as it's
    /// now: a conflict waiting from its first marker to its last, a
    /// resolved one where its text is.
    fn spans(&self, m: &Merging) -> Vec<(usize, u32, u32)> {
        let How::Hunks(parts) = &self.how else { return Vec::new() };
        let count = |t: &str| {
            let n = t.matches('\n').count() + usize::from(!t.is_empty() && !t.ends_with('\n'));
            u32::try_from(n).unwrap_or(u32::MAX)
        };
        let mut line = 1u32;
        let mut out = Vec::new();
        for (i, part) in parts.iter().enumerate() {
            let n = match part {
                Part::Merged(t) => count(t),
                Part::Conflict { .. } => count(&merge::render(
                    std::slice::from_ref(part),
                    &m.head_ref,
                    &m.base_ref,
                )),
            };
            if matches!(part, Part::Conflict { .. }) && n > 0 {
                out.push((i, line, line.saturating_add(n - 1)));
            }
            line = line.saturating_add(n);
        }
        out
    }
}

/// The merge on screen: its diff's files are its conflicts.
fn on_conflicts(state: &State) -> Option<&Merging> {
    let screen = state.screen().diff()?;
    let m = state.merging.as_ref()?;
    (screen.of == m.of()).then_some(m)
}

/// Where `pr`'s conflicts can't be resolved from, and why.
fn why_not(state: &State, pr: &PrRef) -> Result<(), String> {
    let d = state
        .pr(pr)
        .ok_or_else(|| format!("{pr} hasn't loaded yet"))?;
    match d.mergeable {
        Mergeable::Conflicting => {}
        Mergeable::Unknown => return Err("GitHub is still checking whether it conflicts".into()),
        Mergeable::Yes => return Err(format!("It has no conflicts with {}", d.base_ref)),
    }
    crate::suggest::may_push(pr, d)
}

fn act(state: &mut State, action: Action) -> Cmds {
    if action == Action::ResolveConflicts {
        return start(state);
    }
    let Some(m) = on_conflicts(state).cloned() else {
        let start = state.first_key(Action::ResolveConflicts);
        return state.just_say(format!(
            "{}: works on a pull request's conflicts, which {start} opens",
            action.description()
        ));
    };
    match action {
        Action::TakeOurs => pick(state, m, Pick::Ours),
        Action::TakeTheirs => pick(state, m, Pick::Theirs),
        Action::EditComment => edit(state, &m),
        Action::CommitMerge => ask(state, &m),
        Action::AbortMerge => {
            leave(state, &m);
            state.info(format!(
                "Stopped resolving {}'s conflicts: GitHub's branch is as it was",
                m.pr
            ));
            state.load_visible(false)
        }
        _ => Cmds::new(),
    }
}

/// Starts resolving the conflicts of the pull request on screen, or
/// goes back to its merge being resolved.
fn start(state: &mut State) -> Cmds {
    let Some(Subject::Pr(pr)) = Subject::on_screen(state) else {
        return state.just_say("Resolving conflicts works on a pull request");
    };
    if let Err(why) = why_not(state, &pr) {
        return state.just_say(why);
    }
    let Some(d) = state.pr(&pr) else { return Cmds::new() };
    if let Some(m) = state
        .merging
        .as_ref()
        .filter(|m| m.pr == pr && *m.head == d.head_oid)
    {
        let of = m.of();
        if state.screen().diff().is_some_and(|s| s.of == of) {
            return state.just_say("These are its conflicts");
        }
        return state.open_diff(of, None);
    }
    let job = Start {
        pr: pr.clone(),
        base_ref: d.base_ref.clone(),
        head_ref: d.head_ref.clone(),
        head_repo: (d.head_repo.as_deref())
            .and_then(RepoId::parse)
            .unwrap_or_else(|| pr.repo.clone()),
        viewer: state.viewer.clone(),
    };
    state.info(format!(
        "Merging {} into {} with git…",
        job.base_ref, job.head_ref
    ));
    crate::work::start(job)
}

/// The conflict under the cursor (by its file, and its part for one in
/// a text), or why there's none.
fn under(state: &State, m: &Merging) -> Result<(usize, Option<usize>), String> {
    let (Some(screen), Some(diff)) = (state.screen().diff(), state.diff()) else {
        return Err(String::new());
    };
    let next = state.first_key(Action::NextHunk);
    let not_here = format!("Move into a conflict, between its markers ({next} goes to the next)");
    let doc = &diff.doc;
    let Some(path) = (screen.focus == Pane::Diff)
        .then(|| doc.files().get(screen.cursor.file))
        .flatten()
        .map(|f| f.meta.path())
    else {
        return Err(not_here);
    };
    let Some(at) = m.files.iter().position(|f| f.path == path) else {
        return Err(not_here);
    };
    let Some(file) = m.files.get(at) else { return Err(not_here) };
    if matches!(file.how, How::Whole { .. }) {
        return Ok((at, None));
    }
    let line = doc
        .line_at(screen.cursor, Side::Right)
        .filter(|l| l.side == Side::Right);
    let Some(line) = line else { return Err(not_here) };
    let part = file
        .spans(m)
        .into_iter()
        .find(|&(_, first, last)| (first..=last).contains(&line.line));
    part.map(|(part, ..)| (at, Some(part))).ok_or(not_here)
}

/// Resolves the conflict under the cursor with one side.
fn pick(state: &mut State, mut m: Merging, side: Pick) -> Cmds {
    let (at, part) = or_return!(under(state, &m), |why| state.just_say(why));
    let Some(file) = m.files.get_mut(at) else { return Cmds::new() };
    match (&mut file.how, part) {
        (How::Hunks(parts), Some(part)) => {
            if let Some(Part::Conflict {
                ours,
                theirs,
                chosen,
            }) = parts.get_mut(part)
            {
                *chosen = Some(match side {
                    Pick::Ours => ours.clone(),
                    Pick::Theirs => theirs.clone(),
                });
            }
        }
        (How::Whole { chosen, .. }, _) => *chosen = Some(side),
        (How::Hunks(_), None) => return Cmds::new(),
    }
    let whose = match side {
        Pick::Ours => format!("{}'s", m.head_ref),
        Pick::Theirs => format!("{}'s", m.base_ref),
    };
    resolved(state, m, at, &format!("Took {whose} side"))
}

/// Opens the conflict under the cursor in `$EDITOR`.
fn edit(state: &mut State, m: &Merging) -> Cmds {
    let (at, part) = or_return!(under(state, m), |why| state.just_say(why));
    let Some(File { path, how, .. }) = m.files.get(at) else { return Cmds::new() };
    let (How::Hunks(parts), Some(part)) = (how, part) else {
        let (ours, theirs) = (
            state.first_key(Action::TakeOurs),
            state.first_key(Action::TakeTheirs),
        );
        return state.just_say(format!(
            "{path} conflicts as a whole: {ours} keeps your branch's, {theirs} the base's"
        ));
    };
    let Some(one) = parts.get(part) else { return Cmds::new() };
    let text = match one {
        Part::Conflict {
            chosen: Some(t), ..
        } => t.clone(),
        Part::Conflict { chosen: None, .. } => {
            merge::render(std::slice::from_ref(one), &m.head_ref, &m.base_ref)
        }
        Part::Merged(_) => return Cmds::new(),
    };
    Cmd::Edit {
        purpose: EditPurpose::Conflict {
            path: path.clone(),
            part,
        },
        text,
    }
    .into()
}

/// The editor closed on a conflict of `path`: its text is the resolution.
pub(crate) fn edited(state: &mut State, path: &str, part: usize, mut text: String) -> Cmds {
    let Some(mut m) = state.merging.clone() else { return Cmds::new() };
    let Some(at) = m.files.iter().position(|f| f.path == path) else {
        return Cmds::new();
    };
    let Some(File {
        how: How::Hunks(parts),
        ..
    }) = m.files.get_mut(at)
    else {
        return Cmds::new();
    };
    let last = part + 1 == parts.len();
    if !last && !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if merge::parts(&text).is_some() {
        let left = "It still has conflict markers: edit it again, or take a side";
        return state.just_say(left);
    }
    if let Some(Part::Conflict { chosen, .. }) = parts.get_mut(part) {
        *chosen = Some(text);
    }
    resolved(state, m, at, "Edited")
}

/// File `index` of diff `of`, as resolving it has made it so far: a
/// diff loaded again shows what was resolved before.
pub(crate) fn as_resolved(
    state: &State,
    of: &DiffOf,
    index: usize,
) -> Option<(ChangedFile, FileDiff)> {
    let m = state.merging.as_ref().filter(|m| m.of() == *of)?;
    let doc = &state.diffs.get(of)?.doc;
    let was = &doc.files().get(index)?.meta;
    let file = m.files.iter().find(|f| f.path == was.path())?;
    let begun = match &file.how {
        How::Hunks(parts) => (parts.iter()).any(|p| {
            matches!(
                p,
                Part::Conflict {
                    chosen: Some(_),
                    ..
                }
            )
        }),
        How::Whole { chosen, .. } => chosen.is_some(),
    };
    begun.then(|| (file.change(was), file.diff(m)))
}

/// One of file `at`'s conflicts was resolved: the diff shows it, and
/// what's left is said.
fn resolved(state: &mut State, m: Merging, at: usize, done: &str) -> Cmds {
    let Some(path) = m.files.get(at).map(|f| f.path.clone()) else {
        return Cmds::new();
    };
    let left = m.left();
    let of = m.of();
    state.merging = Some(m);
    let index = (state.diffs.get(&of))
        .and_then(|d| d.doc.files().iter().position(|f| f.meta.path() == path));
    let now = index.and_then(|i| Some((i, as_resolved(state, &of, i)?)));
    if let Some((screen, d)) = state.diff_parts()
        && let Some((index, (meta, diff))) = now
    {
        diff_screen::preserving_position(screen, d, |d| {
            d.set_change(index, meta, Arc::new(diff));
        });
    }
    let what = match left {
        0 => {
            let commit = state.first_key(Action::CommitMerge);
            format!("all resolved: {commit} commits the merge")
        }
        n => format!("{} left", count(n, "conflict")),
    };
    state.just_say(format!("{done} · {what}"))
}

/// Asks before committing the merge and pushing it.
fn ask(state: &mut State, m: &Merging) -> Cmds {
    let left = m.left();
    if left > 0 {
        let (ours, theirs) = (
            state.first_key(Action::TakeOurs),
            state.first_key(Action::TakeTheirs),
        );
        let edit = state.first_key(Action::EditComment);
        return state.just_say(format!(
            "{} left: {ours} takes your branch's side, {theirs} the base's, {edit} edits",
            count(left, "conflict")
        ));
    }
    let files: Vec<(String, Content)> = (m.files.iter())
        .filter_map(|f| Some((f.path.clone(), f.content(m)?)))
        .collect();
    let n = files.len();
    let facts = vec![
        (format!("“{}”", m.message()), false),
        (format!("{} resolved", count(n, "file")), false),
        (format!("Pushed to {}:{}", m.head_repo, m.head_ref), false),
        (
            format!("Not pushed if it moved past {}", short_sha(&m.head)),
            false,
        ),
    ];
    let job = Finish {
        pr: m.pr.clone(),
        head_repo: m.head_repo.clone(),
        head_ref: m.head_ref.clone(),
        head: m.head.clone(),
        base: m.base.clone(),
        tree: m.tree.clone(),
        files,
        message: m.message(),
        viewer: state.viewer.clone(),
    };
    let title = format!("Commit the merge and push it to {}?", m.head_ref);
    let choices = vec![(
        "Commit merge".to_owned(),
        Answer::CommitMerge(Box::new(job)),
    )];
    crate::forms::ask(state, Route::pr(m.pr.clone()), title, facts, choices)
}

/// The question was answered: the merge is committed and pushed.
pub(crate) fn send(state: &mut State, job: Finish) -> Cmds {
    crate::forms::sending(state);
    crate::work::start(job)
}

/// The merge is done with: its diff closes, without a way forward to it.
fn leave(state: &mut State, m: &Merging) {
    let of = m.of();
    state.merging = None;
    state.diffs.remove(&of);
    let ours = |s: &Screen| matches!(s, Screen::Diff(d) if d.of == of);
    state.forward.retain(|s| !ours(s));
    if ours(state.screen()) {
        let _ = state.screens.pop();
    }
}

/// The menu's rows: starting where a pull request conflicts, and on its
/// conflicts' diff, what resolves them.
fn offers(state: &State, out: &mut Vec<Doable>) {
    if let Some(m) = on_conflicts(state) {
        let why = under(state, m).err();
        let side = |action, label, short| row("Resolve", action, label, short, why.clone());
        out.push(side(
            Action::TakeOurs,
            format!("Take {}'s side (yours)", m.head_ref),
            "yours",
        ));
        out.push(side(
            Action::TakeTheirs,
            format!("Take {}'s side", m.base_ref),
            "theirs",
        ));
        out.push(side(
            Action::EditComment,
            "Edit the conflict in $EDITOR".to_owned(),
            "edit",
        ));
        let left = m.left();
        let why = (left > 0).then(|| format!("{} left", count(left, "conflict")));
        out.push(row(
            "Resolve",
            Action::CommitMerge,
            "Commit the merge and push it",
            "",
            why,
        ));
        let stop = "Stop resolving, leaving GitHub as it was";
        out.push(row("Resolve", Action::AbortMerge, stop, "", None));
        return;
    }
    let Some(Subject::Pr(pr)) = Subject::on_screen(state) else { return };
    let Some(d) = state.pr(&pr) else { return };
    if d.mergeable != Mergeable::Conflicting {
        return;
    }
    let label = format!("Resolve the conflicts with {}", d.base_ref);
    let why = why_not(state, &pr).err();
    out.push(row("Change", Action::ResolveConflicts, label, "", why));
}

/// Merges a pull request's base into its head with git, to resolve
/// their conflicts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub pr: PrRef,
    pub base_ref: String,
    pub head_ref: String,
    pub head_repo: RepoId,
    /// Your login: the look commit's author when git has no name for you.
    pub viewer: Option<String>,
}

/// Where GitHub's tips are fetched to, to merge.
const HEAD: &str = "refs/ghtui/merge/head";
const BASE: &str = "refs/ghtui/merge/base";

impl Job for Start {
    /// None: it merges without conflicts.
    type Out = Option<Merging>;
    const WRITES: bool = false;

    async fn run(&self, cx: &Cx) -> Result<Option<Merging>, Failure> {
        let version = crate::git::version().await?;
        if version < merge::MERGE_TREE {
            return Err(Failure::msg(format!(
                "Resolving conflicts takes git {} or newer: this is {version}",
                merge::MERGE_TREE
            )));
        }
        let base = (&*self.pr.repo.owner, &*self.pr.repo.name);
        let repo = cx.git.repo(base.0, base.1, &|_| {}).await?;
        let head = (&*self.head_repo.owner, &*self.head_repo.name);
        let remote = branch::remote_for(&repo, base, head).await;
        let fetch = |branch: &str, e: &crate::git::GitError| {
            Failure::msg(format!("Couldn't fetch {branch}: {}", refused(e)))
        };
        let ours = (branch::tip(&repo, &remote, &self.head_ref, HEAD).await)
            .map_err(|e| fetch(&self.head_ref, &e))?;
        let theirs = (branch::tip(&repo, &repo.remote, &self.base_ref, BASE).await)
            .map_err(|e| fetch(&self.base_ref, &e))?;
        let merged = merge::merge(&repo, &ours, &theirs).await?;
        if merged.conflicted.is_empty() {
            return Ok(None);
        }
        let mut files = Vec::new();
        for c in merged.conflicted {
            let side = async |e: Option<Entry>| -> Result<_, Failure> {
                let Some(e) = e else { return Ok(None) };
                let bytes = merge::read(&repo, &e.oid).await?;
                Ok(Some((e, bytes)))
            };
            let at_head = format!("{ours}:{}", c.path);
            let head_file = repo.run_bytes(&["cat-file", "blob", &at_head]).await.ok();
            let marked = match &c.merged {
                Some(e) => String::from_utf8(merge::read(&repo, &e.oid).await?).ok(),
                None => None,
            };
            let how = match marked.as_deref().and_then(merge::parts) {
                Some(parts) => How::Hunks(parts),
                None => How::Whole {
                    ours: side(c.ours).await?,
                    theirs: side(c.theirs).await?,
                    chosen: None,
                },
            };
            files.push(File {
                path: c.path,
                ours: head_file,
                how,
            });
        }
        let mut m = Merging {
            pr: self.pr.clone(),
            base_ref: self.base_ref.clone(),
            head_ref: self.head_ref.clone(),
            head_repo: self.head_repo.clone(),
            head: ours.clone(),
            base: theirs,
            tree: merged.tree,
            view: ours.clone(),
            files,
        };
        // The look: the head, with each conflicted file as it is now.
        let look: Vec<(String, Content)> = (m.files.iter())
            .map(|f| {
                let now = match &f.how {
                    How::Hunks(parts) => {
                        Content::Text(merge::render(parts, &m.head_ref, &m.base_ref))
                    }
                    How::Whole {
                        theirs: Some((e, _)),
                        ..
                    } => Content::Entry(e.clone()),
                    How::Whole { .. } => Content::Gone,
                };
                (f.path.clone(), now)
            })
            .collect();
        let message = format!("Conflicts of {} to resolve", self.pr);
        m.view = merge::commit(
            &repo,
            &ours,
            &[&ours],
            &look,
            &message,
            self.viewer.as_deref(),
        )
        .await?;
        Ok(Some(m))
    }

    fn done(self, state: &mut State, result: Result<Option<Merging>, Failure>) -> Cmds {
        match result {
            Ok(Some(m)) => {
                let of = m.of();
                let n = m.left();
                state.merging = Some(m);
                let cmds = state.open_diff(of, None);
                // The diff's keys, now that it's on screen.
                let (ours, theirs) = (
                    state.first_key(Action::TakeOurs),
                    state.first_key(Action::TakeTheirs),
                );
                let edit = state.first_key(Action::EditComment);
                state.info(format!(
                    "{} to resolve: {ours} takes your branch's side, {theirs} the base's, {edit} edits",
                    count(n, "conflict")
                ));
                cmds
            }
            Ok(None) => {
                let update = state.first_key(Action::UpdateBranch);
                state.just_say(format!(
                    "{} merges into {} without conflicts now: {update} updates the branch",
                    self.base_ref, self.head_ref
                ))
            }
            Err(why) => {
                state.error(format!(
                    "Couldn't merge {} into {}: {why}",
                    self.base_ref, self.head_ref
                ));
                Cmds::new()
            }
        }
    }
}

/// Commits a resolved merge and pushes it to the head branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finish {
    pub pr: PrRef,
    pub head_repo: RepoId,
    pub head_ref: String,
    pub head: Oid,
    pub base: Oid,
    pub tree: Oid,
    pub files: Vec<(String, Content)>,
    pub message: String,
    pub viewer: Option<String>,
}

impl Job for Finish {
    type Out = Oid;

    async fn run(&self, cx: &Cx) -> Result<Oid, Failure> {
        let base = (&*self.pr.repo.owner, &*self.pr.repo.name);
        let repo = cx.git.repo(base.0, base.1, &|_| {}).await?;
        let head = (&*self.head_repo.owner, &*self.head_repo.name);
        let remote = branch::remote_for(&repo, base, head).await;
        let parents = [&*self.head, &*self.base];
        let viewer = self.viewer.as_deref();
        let made = merge::commit(
            &repo,
            &self.tree,
            &parents,
            &self.files,
            &self.message,
            viewer,
        )
        .await?;
        match branch::push(&repo, &remote, &made, &self.head_ref).await {
            Ok(()) => Ok(made),
            Err(err) if commit::moved_on(&err) => Err(Failure::msg(format!(
                "{} moved on GitHub since the merge was made: resolve its conflicts again",
                self.head_ref
            ))),
            Err(err) => Err(Failure::msg(format!(
                "GitHub refused the push: {}",
                refused(&err)
            ))),
        }
    }

    /// Pushed: the conflicts' diff closes, and the pull request follows
    /// GitHub until it shows the merge as its head.
    fn done(self, state: &mut State, result: Result<Oid, Failure>) -> Cmds {
        let route = Route::pr(self.pr.clone());
        let made = or_return!(result, |why| {
            crate::forms::failed(state, &route, format!("Not pushed: {why}"))
        });
        crate::forms::close_sender(state, &route);
        if let Some(m) = state.merging.clone().filter(|m| m.pr == self.pr) {
            leave(state, &m);
        }
        state.info(format!(
            "Merged into {} and pushed as {}",
            self.head_ref,
            short_sha(&made)
        ));
        // The pull request's page it was started from, else its conversation.
        let here = state.route().filter(
            |r| matches!(r, Route::Pulls(crate::route::pulls::Pr { pr, .. }) if *pr == self.pr),
        );
        let route = here.cloned().unwrap_or(route);
        let pushed = Pushed {
            pr: self.pr.clone(),
            head: made,
        };
        crate::follow::expect(state, route, pushed);
        let cmds = state.load_visible(false);
        cmds.and(state.ensure(crate::data::DataKey::Pr(self.pr), true))
    }
}

/// A pull request shows the merge pushed to it once it's its head.
#[derive(Debug)]
struct Pushed {
    pr: PrRef,
    head: Oid,
}

impl Expectation for Pushed {
    fn waiting(&self) -> &'static str {
        "Waiting for GitHub to show the merge…"
    }

    fn shown(&self, state: &State, _: &Route) -> Shown {
        state
            .pr(&self.pr)
            .is_some_and(|d| *d.head_oid == *self.head)
            .into()
    }
}
