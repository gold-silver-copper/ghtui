//! Editing a file and committing it, as the website's pencil does; and
//! making, renaming and deleting one. The form reads the file as GitHub
//! has it on the branch, and the composer (or `$EDITOR`) writes it. The
//! commit goes to the branch you're on, or to a new one, offered as a
//! pull request. In your clone git makes it, after asking, and pushes it;
//! elsewhere GitHub's API does. A file changed on GitHub since you opened
//! it is never overwritten: committing says so, and keeps your text.

use crate::api::GitHub;
use crate::api::housekeeping::{FileCommit, FileLook, RefKind};
use crate::api::model::RepoId;
use crate::create::Look;
use crate::route::{Route, code, pulls};
use crate::ui::page::{Link, Page};
use crate::ui::pages::{Mark, TextBox, WriteFormView};

use super::{
    Chosen, Expected, Field, FileChange, FileWrite, Form, FormOf, Subject, Work, ask, clone_here,
    expect, failed, form, form_mut, form_row, item, refused,
};
use crate::keymap::Action;
use crate::nav::Doable;
use crate::state::{Cmd, State};
use crate::writes::{Work as Writes, row};

const WHAT: &str = "Changing its files";

/// Why the repository's files can't be changed here, with what to do.
fn why_not(state: &State, repo: &RepoId) -> Option<String> {
    refused(state, repo, WHAT).map(|why| {
        let fork = state.first_key(Action::Fork);
        format!("{why}: fork it ({fork}) and change your fork, as GitHub would")
    })
}

/// `,e`, `,D` on a file, `,o` in a folder: the file's form.
#[must_use]
pub(super) fn open(
    state: &mut State,
    repo: RepoId,
    rev: String,
    path: String,
    change: FileChange,
) -> Vec<Cmd> {
    if let Some(why) = why_not(state, &repo) {
        state.info(why);
        return Vec::new();
    }
    super::open(
        state,
        FormOf::File {
            repo,
            rev,
            path,
            change,
        },
    )
}

/// The file's form's parts.
fn parts(of: &FormOf) -> Option<(&RepoId, &str, &str, FileChange)> {
    match of {
        FormOf::File {
            repo,
            rev,
            path,
            change,
        } => Some((repo, rev, path, *change)),
        _ => None,
    }
}

/// What the form has read from GitHub.
fn looked(f: &Form) -> Option<&FileLook> {
    match &f.look {
        Some(Look::Found(look)) => Some(look),
        _ => None,
    }
}

/// Reads the file from GitHub, once; then fills the form and opens the
/// composer on its text.
#[must_use]
pub(super) fn follow(state: &mut State, route: &Route, of: &FormOf) -> Vec<Cmd> {
    let Some((repo, rev, path, change)) = parts(of) else {
        return Vec::new();
    };
    let f = form(state, route);
    if f.look.is_none() {
        let f = form_mut(state, route);
        f.look = Some(Look::Asking);
        match change {
            FileChange::New if path.is_empty() => f.set(Field::Path, ""),
            FileChange::New => f.set(Field::Path, format!("{path}/")),
            FileChange::Edit | FileChange::Delete => f.set(Field::Path, path),
        }
        return vec![Cmd::Write(Writes::Housekeeping(Work::FileLook {
            route: route.clone(),
            repo: repo.clone(),
            rev: rev.to_owned(),
            path: path.to_owned(),
        }))];
    }
    if f.started || looked(f).is_none() {
        return Vec::new();
    }
    let look = looked(f).cloned();
    let login = state.viewer.clone().unwrap_or_else(|| "patch".into());
    let f = form_mut(state, route);
    f.started = true;
    let Some(look) = look else {
        return Vec::new();
    };
    if change == FileChange::Edit {
        f.text = look.file.and_then(|file| file.text).unwrap_or_default();
    }
    // A tag or a commit isn't a branch: the change goes on a new one.
    if look.branch_head.is_none() {
        f.set(Field::Branch, format!("{login}-patch-1"));
    }
    state.shown_gen += 1;
    if state.overlay.is_some() {
        return Vec::new();
    }
    match change {
        FileChange::New => super::open_field(state, route, Field::Path),
        FileChange::Edit if problem(state, route, of).is_none_or(|why| !why.contains("binary")) => {
            super::compose(state)
        }
        _ => Vec::new(),
    }
}

/// GitHub's answer to where the file starts.
#[must_use]
pub(super) fn on_looked(
    state: &mut State,
    route: &Route,
    result: Result<FileLook, String>,
) -> Vec<Cmd> {
    let look = match result {
        Ok(look) => Look::Found(look),
        Err(why) => Look::Failed(why),
    };
    form_mut(state, route).look = Some(look);
    state.shown_gen += 1;
    if state.route() == Some(route) {
        return super::follow(state);
    }
    Vec::new()
}

/// Why `path` can't name a file in a repository, if it can't.
pub(super) fn path_problem(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split('/').collect();
    if path.is_empty() || path.ends_with('/') {
        Some("Name the file".into())
    } else if path.starts_with('/') || parts.iter().any(|p| p.is_empty()) {
        Some("A path's folders are separated by one /, with none first".into())
    } else if parts.iter().any(|p| matches!(*p, "." | ".." | ".git")) {
        Some("A path can't have ., .. or .git in it".into())
    } else if path.chars().any(char::is_control) {
        Some("A path can't have control characters".into())
    } else {
        None
    }
}

/// The path the form writes.
fn path(f: &Form) -> &str {
    f.get(Field::Path).trim()
}

/// Why the commit can't be made yet, if it can't.
pub(super) fn problem(state: &State, route: &Route, of: &FormOf) -> Option<String> {
    let (repo, rev, _, change) = parts(of)?;
    if let Some(why) = why_not(state, repo) {
        return Some(why);
    }
    let f = form(state, route);
    let look = match &f.look {
        None | Some(Look::Asking) => return Some("Still reading the file from GitHub".into()),
        Some(Look::Failed(why)) => return Some(format!("Couldn't read it from GitHub: {why}")),
        Some(Look::Found(look)) => look,
    };
    match (change, &look.file) {
        (FileChange::Edit | FileChange::Delete, None) => {
            return Some(format!("{rev} has no such file"));
        }
        (FileChange::Edit, Some(file)) if file.text.is_none() => {
            return Some(
                "It's binary, or too big for GitHub's API: it can't be edited here".into(),
            );
        }
        _ => {}
    }
    if let Some(why) = path_problem(path(f)) {
        return Some(why);
    }
    let branch = f.get(Field::Branch).trim();
    if branch.is_empty() && look.branch_head.is_none() {
        return Some(format!("{rev} isn't a branch: commit to a new one"));
    }
    if !branch.is_empty()
        && let Some(why) = super::refs::name_problem(branch)
    {
        return Some(why.into());
    }
    None
}

/// The commit's message: what's written, else GitHub's own.
fn message(f: &Form, of: &FormOf) -> String {
    let written = f.get(Field::Message).trim();
    if !written.is_empty() {
        return written.to_owned();
    }
    let Some((_, _, was, change)) = parts(of) else {
        return String::new();
    };
    let now = path(f);
    let name = |p: &str| p.rsplit('/').next().unwrap_or(p).to_owned();
    match change {
        FileChange::New => format!("Create {}", name(now)),
        FileChange::Delete => format!("Delete {}", name(was)),
        FileChange::Edit if was != now => format!("Rename {} to {}", name(was), name(now)),
        FileChange::Edit => format!("Update {}", name(now)),
    }
}

/// The branch picker's rows: the branch it's on, or a new one.
pub(super) fn branch_rows(
    state: &State,
    route: &Route,
    q: &str,
) -> Vec<(
    crate::ui::overlays::PaletteItem,
    Option<crate::writes::Chosen>,
)> {
    let Route::Housekeeping(of) = route else {
        return Vec::new();
    };
    let Some((_, rev, _, _)) = parts(of) else {
        return Vec::new();
    };
    let set = |b: &str| Some(Chosen::Field(route.clone(), Field::Branch, b.to_owned()));
    let f = form(state, route);
    let mut rows = Vec::new();
    if !q.is_empty() {
        rows.push(match super::refs::name_problem(q) {
            Some(why) => item(why, "", None),
            None => item(
                format!("A new branch, {q}"),
                "and offer a pull request",
                set(q),
            ),
        });
    }
    if looked(f).is_some_and(|l| l.branch_head.is_some()) {
        rows.push(item(format!("Commit directly to {rev}"), "", set("")));
    }
    let login = state.viewer.as_deref().unwrap_or("patch");
    let suggested = format!("{login}-patch-1");
    if q.is_empty() {
        rows.push(item(
            format!("A new branch, {suggested}"),
            "and offer a pull request",
            set(&suggested),
        ));
    }
    rows
}

/// `,s`: asks before committing, as GitHub's "Commit changes" does.
#[must_use]
pub(super) fn ask_commit(state: &mut State, route: &Route) -> Vec<Cmd> {
    let Route::Housekeeping(of) = route else {
        return Vec::new();
    };
    let Some((repo, rev, was, change)) = parts(of) else {
        return Vec::new();
    };
    let repo = repo.clone();
    if let Some(why) = problem(state, route, of) {
        crate::writes::say(state, why);
        return Vec::new();
    }
    let clone = match clone_here(state, &repo) {
        Ok(clone) => clone,
        Err(cmds) => return cmds,
    };
    let f = form(state, route);
    let now = path(f).to_owned();
    let branch = f.get(Field::Branch).trim().to_owned();
    let mut facts = vec![(format!("“{}”", message(f, of)), false)];
    facts.push((
        match change {
            FileChange::New => format!("Creates {now}"),
            FileChange::Delete => format!("Deletes {was}"),
            FileChange::Edit if was != now => format!("Renames {was} to {now}"),
            FileChange::Edit => format!("Changes {now}"),
        },
        change == FileChange::Delete,
    ));
    facts.push((
        if branch.is_empty() {
            format!("Commits to {rev}")
        } else {
            format!("Commits to a new branch, {branch} (from {rev})")
        },
        false,
    ));
    facts.push((
        match &clone {
            Some(c) => format!(
                "{}; your clone's branches and files stay as they are",
                super::refs::in_clone(c)
            ),
            None => "On GitHub, by you".to_owned(),
        },
        false,
    ));
    if change != FileChange::New {
        facts.push((
            "If GitHub's file changed since you opened it, nothing is overwritten".into(),
            false,
        ));
    }
    let title = match change {
        FileChange::Delete => format!("Delete {was}?"),
        _ => "Commit changes?".to_owned(),
    };
    let button = match change {
        FileChange::Delete => "Delete and commit",
        _ => "Commit changes",
    };
    ask(
        state,
        None,
        title,
        facts,
        vec![(button.into(), super::Answer::Commit)],
    );
    Vec::new()
}

/// The commit asked for, sent.
#[must_use]
pub(super) fn commit(state: &mut State, route: &Route) -> Vec<Cmd> {
    let Route::Housekeeping(of) = route else {
        return Vec::new();
    };
    let Some((repo, rev, was, change)) = parts(of) else {
        return Vec::new();
    };
    if let Some(why) = problem(state, route, of) {
        crate::writes::say(state, why);
        return Vec::new();
    }
    let f = form(state, route);
    let Some(look) = looked(f) else {
        return Vec::new();
    };
    let now = path(f).to_owned();
    let new_branch = f.get(Field::Branch).trim().to_owned();
    let seen_file = look.file.as_ref().map(|file| file.oid.clone());
    let (seen, additions, deletions) = match change {
        FileChange::New => (
            vec![(now.clone(), None)],
            vec![(now, f.text.clone())],
            Vec::new(),
        ),
        FileChange::Delete => (
            vec![(was.to_owned(), seen_file)],
            Vec::new(),
            vec![was.to_owned()],
        ),
        FileChange::Edit if was != now => (
            vec![(was.to_owned(), seen_file), (now.clone(), None)],
            vec![(now, f.text.clone())],
            vec![was.to_owned()],
        ),
        FileChange::Edit => (
            vec![(now.clone(), seen_file)],
            vec![(now, f.text.clone())],
            Vec::new(),
        ),
    };
    let (branch, from, head) = if new_branch.is_empty() {
        let head = look.branch_head.clone().unwrap_or_default();
        (rev.to_owned(), None, head)
    } else {
        (new_branch, Some(look.commit.clone()), look.commit.clone())
    };
    let clone = match state.creating.clones.get(repo) {
        Some(Look::Found(clone)) => clone.clone(),
        _ => None,
    };
    let base = look.branch_head.is_some().then(|| rev.to_owned());
    let write = FileWrite {
        repo: repo.clone(),
        base,
        commit: super::git::Commit {
            branch,
            from,
            seen,
            additions,
            deletions,
            message: message(f, of),
            login: state.viewer.clone(),
        },
        head,
        clone,
    };
    form_mut(state, route).sending = true;
    crate::writes::sending(state);
    vec![Cmd::Write(Writes::Housekeeping(Work::Commit(
        route.clone(),
        Box::new(write),
    )))]
}

/// What committing says when GitHub's file changed since it was read.
fn changed_meanwhile(path: &str) -> String {
    format!(
        "{path} changed on GitHub since you opened it: committing would overwrite that, so it isn't committed. Your text is kept here; open the file again to see GitHub's"
    )
}

/// Commits it: with git in your clone, or with GitHub's API, on a new
/// branch made first when it's one.
pub(super) async fn run(gh: &GitHub, w: &FileWrite) -> Result<(), String> {
    let c = &w.commit;
    if let Some(clone) = &w.clone {
        return super::git::commit(&clone.top, &clone.remote, c)
            .await
            .map(drop);
    }
    let why = |err: crate::api::ApiError| crate::writes::reason(&err);
    let head = match &c.from {
        Some(from) => {
            gh.create_ref(&w.repo, RefKind::Branch, &c.branch, from)
                .await
                .map_err(|e| format!("GitHub didn't make the branch {}: {}", c.branch, why(e)))?;
            from.clone()
        }
        None => {
            // GitHub's branch now: committed on, unless a file changed.
            let mut head = w.head.clone();
            for (path, seen) in &c.seen {
                let look = gh.file_look(&w.repo, &c.branch, path).await.map_err(why)?;
                if look.file.map(|f| f.oid) != *seen {
                    return Err(match seen {
                        Some(_) => changed_meanwhile(path),
                        None => format!("{} has {path} already", c.branch),
                    });
                }
                head = look.branch_head.unwrap_or(head);
            }
            head
        }
    };
    let (headline, body) = match c.message.split_once('\n') {
        Some((headline, body)) => (headline.trim().to_owned(), body.trim().to_owned()),
        None => (c.message.trim().to_owned(), String::new()),
    };
    let commit = FileCommit {
        repo: w.repo.clone(),
        branch: c.branch.clone(),
        head,
        headline,
        body,
        additions: c.additions.clone(),
        deletions: c.deletions.clone(),
    };
    match gh.commit_files(&commit).await {
        Ok(()) => Ok(()),
        Err(err) => {
            // A new branch with nothing on it isn't left behind.
            if c.from.is_some() {
                let _ = gh.delete_ref(&w.repo, RefKind::Branch, &c.branch).await;
            }
            let reason = why(err);
            if reason.contains("Expected branch to point to") {
                return Err(format!(
                    "{} moved on GitHub while committing: commit again",
                    c.branch
                ));
            }
            Err(reason)
        }
    }
}

/// Committed: the file (a deleted one's folder) opens at the branch, and
/// follows GitHub until it shows the change; a new branch is offered as a
/// pull request.
#[must_use]
pub(super) fn on_committed(
    state: &mut State,
    route: &Route,
    write: Box<FileWrite>,
    result: Result<(), String>,
) -> Vec<Cmd> {
    let Route::Housekeeping(of) = route else {
        return Vec::new();
    };
    let Some((repo, _, _, change)) = parts(of) else {
        return Vec::new();
    };
    let repo = repo.clone();
    if let Err(why) = result {
        failed(state, route, why);
        return Vec::new();
    }
    state.writing.housekeeping.forms.remove(route);
    crate::writes::close_sender(state, route);
    let FileWrite {
        commit: c, base, ..
    } = *write;
    state.info(format!("Committed to {}", c.branch));
    if state.route() != Some(route) {
        return Vec::new();
    }
    let (page, text, path) = match (change, c.additions.first()) {
        (FileChange::Delete, _) | (_, None) => {
            let path = c.deletions.first().cloned().unwrap_or_default();
            let dir = path.rsplit_once('/').map_or("", |(dir, _)| dir).to_owned();
            let page = Route::Code(code::Tree {
                repo: repo.clone(),
                rev: c.branch.clone(),
                path: dir,
            });
            (page, None, path)
        }
        (_, Some((path, text))) => {
            let page = Route::blob(repo.clone(), c.branch.clone(), path.clone());
            (page, Some(text.clone()), path.clone())
        }
    };
    let mut cmds = state.replace(page.clone(), true, None);
    expect(
        state,
        page,
        Expected::File {
            repo: repo.clone(),
            rev: c.branch.clone(),
            path,
            text,
        },
    );
    if c.from.is_some() {
        ask(
            state,
            Some(Subject::Propose(repo, c.branch.clone(), base)),
            format!("Open a pull request for {}?", c.branch),
            vec![(
                "Its title and description are written on the next page".into(),
                false,
            )],
            vec![("Open a pull request".into(), super::Answer::Propose)],
        );
    }
    cmds.extend(super::follow(state));
    cmds
}

/// A new pull request from `branch`, into the default branch.
#[must_use]
pub(super) fn propose(
    state: &mut State,
    repo: RepoId,
    branch: String,
    base: Option<String>,
) -> Vec<Cmd> {
    let route = Route::Pulls(pulls::NewPr(repo));
    let form = state.creating.forms.entry(route.clone()).or_default();
    form.head = Some(branch);
    // Into the branch it was made from; the default branch is the form's
    // own start.
    if base.is_some() {
        form.base = base;
    }
    form.branch = None;
    let mut cmds = state.push(route);
    cmds.extend(crate::create::follow(state));
    cmds
}

/// The menu's rows on a repository's files.
pub(super) fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    let Some(repo) = route.repo() else {
        return;
    };
    let why = why_not(state, repo);
    match route {
        Route::Code(code::Blob { .. }) => {
            out.push(row(
                "Edit",
                Action::EditComment,
                "Edit this file",
                "edit",
                why.clone(),
            ));
            out.push(row(
                "Edit",
                Action::DeleteComment,
                "Delete this file",
                "",
                why.clone(),
            ));
            out.push(row(
                "New",
                Action::NewHere,
                "New file in this folder",
                "",
                why,
            ));
        }
        _ => out.push(row("New", Action::NewHere, "New file here", "", why)),
    }
}

/// The form's page.
pub(super) fn build(state: &State, page: &mut Page, route: &Route, of: &FormOf) {
    let Some((repo, rev, was, change)) = parts(of) else {
        return;
    };
    let f = form(state, route);
    let heading = match change {
        FileChange::Edit => format!("Editing {was} in {repo}"),
        FileChange::Delete => format!("Deleting {was} from {repo}"),
        FileChange::New => format!("New file in {repo}"),
    };
    let mut status = Vec::new();
    match &f.look {
        None | Some(Look::Asking) => {
            status.push((format!("Reading {rev} from GitHub…"), Mark::Waiting));
        }
        Some(Look::Failed(why)) => status.push((format!("Couldn't read it: {why}"), Mark::Problem)),
        Some(Look::Found(look)) => {
            let at = crate::ui::text::short_sha(&look.commit);
            let what = if change == FileChange::New {
                format!("On {rev} as GitHub had it at {at}")
            } else {
                format!(
                    "As GitHub had it on {rev} at {at}: if it changes meanwhile, committing says so"
                )
            };
            status.push((what, Mark::Fine));
        }
    }
    match state.creating.clones.get(repo) {
        Some(Look::Found(Some(c))) => status.push((
            format!(
                "Will be committed with git in your clone ({}) and pushed",
                c.top.display()
            ),
            Mark::Fine,
        )),
        Some(Look::Asking) => status.push(("Looking for your clone…".into(), Mark::Waiting)),
        _ => status.push(("Will be committed on GitHub, as you".into(), Mark::Fine)),
    }
    let composing = matches!(
        &state.overlay,
        Some(crate::state::Overlay::Compose(c))
            if c.target == crate::review::ComposeTarget::Write(crate::writes::Text::Housekeeping(route.clone()))
    );
    let now = path(f).to_owned();
    let hint = "ctrl-e for $EDITOR";
    let body = if composing { "" } else { f.text.as_str() };
    let boxes = match change {
        FileChange::Delete => Vec::new(),
        _ => vec![TextBox {
            title: now.clone(),
            untitled: "New file",
            body,
            markdown: crate::ui::pages::is_markdown(&now),
            empty: "Nothing written yet.",
            hint,
            link: Link::Comment,
            actions: Vec::new(),
        }],
    };
    let write = state.first_key(Action::Comment);
    let save_key = state.first_key(Action::Create);
    // A deleted file's path is what it is.
    let mut path_row = form_row(f, Field::Path, "Name your file…");
    if change == FileChange::Delete {
        path_row.link = None;
    }
    let mut message_row = form_row(f, Field::Message, "");
    if f.get(Field::Message).trim().is_empty() {
        message_row.value = message(f, of);
        message_row.unset = true;
    }
    let mut branch_row = form_row(f, Field::Branch, "");
    if f.get(Field::Branch).trim().is_empty() {
        branch_row.value = format!("{rev} (directly)");
        branch_row.unset = false;
    } else {
        branch_row.value = format!(
            "a new branch, {}, then a pull request",
            f.get(Field::Branch).trim()
        );
    }
    let view = WriteFormView {
        heading,
        base: Some(crate::ui::markdown::LinkBase::new(repo, rev, "")),
        rows: vec![path_row, branch_row, message_row],
        status,
        boxes,
        after: Vec::new(),
        write: &write,
        button: super::button(of, f),
        save_key: &save_key,
        why_not: problem(state, route, of),
        error: f.error.as_deref(),
    };
    crate::ui::pages::write_form(page, &view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_a_repositorys() {
        assert_eq!(path_problem("src/main.rs"), None);
        assert!(path_problem("src/").is_some());
        assert!(path_problem("/abs").is_some());
        assert!(path_problem("a//b").is_some());
        assert!(path_problem("a/../b").is_some());
        assert!(path_problem(".git/config").is_some());
    }
}
