//! Branches and tags: made at a branch, a tag or a commit, and deleted,
//! from the branches and tags lists. In your clone, git pushes them,
//! after asking; elsewhere GitHub's API makes and deletes them. The
//! default branch, and a branch GitHub's rules protect, aren't deleted.

use crate::api::browse::{BranchInfo, Results, TagInfo};
use crate::api::housekeeping::{BranchRules, RefKind};
use crate::api::model::RepoId;
use crate::git::local::Clone;
use crate::picker::fuzzy_score;
use crate::route::{Route, code};

use super::{
    Chosen, Expected, Pick, RefWrite, Subject, ask, clone_here, expect, item, refused, selected,
};
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::state::{Cmd, State};
use crate::work::{self, Cx, Failure, Job};

/// A branch's or tag's change: made at a commit, or deleted, with git in
/// your clone (`clone`) or with GitHub's API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub repo: RepoId,
    pub kind: RefKind,
    pub name: String,
    pub write: RefWrite,
    pub clone: Option<Clone>,
}

/// What making or deleting one takes.
fn what(kind: RefKind) -> &'static str {
    match kind {
        RefKind::Branch => "Changing branches",
        RefKind::Tag => "Changing tags",
    }
}

/// The branch or tag selected in its list.
fn picked(state: &State, kind: RefKind) -> Option<(String, Option<&BranchInfo>)> {
    let repo = state.route().and_then(Route::repo)?.clone();
    let url = |name: &str| crate::ui::pages::url::tree(&repo, name, "");
    match kind {
        RefKind::Branch => {
            let list = state.picked::<Results<BranchInfo>>(&DataKey::Branches(repo.clone()))?;
            let b = selected(state, &list.items, |b| url(&b.name))?;
            Some((b.name.clone(), Some(b)))
        }
        RefKind::Tag => {
            let list = state.picked::<Results<TagInfo>>(&DataKey::Tags(repo.clone()))?;
            let t = selected(state, &list.items, |t| url(&t.name))?;
            Some((t.name.clone(), None))
        }
    }
}

/// `,o` on the branches or tags: its name first.
#[must_use]
pub(super) fn start(state: &mut State, repo: &RepoId, kind: RefKind) -> Vec<Cmd> {
    if let Some(why) = refused(state, repo, what(kind)) {
        state.info(why);
        return Vec::new();
    }
    let pick = crate::writes::Pick::Housekeeping(Pick::RefName(repo.clone(), kind));
    crate::writes::open_picker(state, pick)
}

/// Why `name` can't name a branch or tag, as git's `check-ref-format`
/// has it.
pub(super) fn name_problem(name: &str) -> Option<&'static str> {
    let bad = |c: char| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c);
    if name.is_empty() {
        Some("Type its name")
    } else if name.chars().any(bad) || name.contains("..") || name.contains("@{") || name == "@" {
        Some("A name can't have spaces, .., @{, or any of ~^:?*[\\")
    } else if name.starts_with(['/', '.', '-'])
        || name.ends_with(['/', '.'])
        || name.ends_with(".lock")
        || name.contains("//")
        || name.split('/').any(|part| part.starts_with('.'))
    {
        Some("A name can't start with / . or -, or end with / . or .lock")
    } else {
        None
    }
}

/// The name picker's one row.
pub(super) fn name_rows(
    repo: &RepoId,
    kind: RefKind,
    q: &str,
) -> Vec<(
    crate::ui::overlays::PaletteItem,
    Option<crate::writes::Chosen>,
)> {
    let q = q.trim();
    match name_problem(q) {
        Some(why) => vec![item(why, "", None)],
        None => vec![item(
            format!("Create {} {q}", kind.noun()),
            "then where it starts",
            Some(Chosen::RefName(repo.clone(), kind, q.to_owned())),
        )],
    }
}

/// Named: where it starts.
#[must_use]
pub(super) fn named(state: &mut State, repo: RepoId, kind: RefKind, name: String) -> Vec<Cmd> {
    let pick = crate::writes::Pick::Housekeeping(Pick::RefFrom(repo, kind, name));
    crate::writes::open_picker(state, pick)
}

/// Where it starts: a commit typed, then the branches (the one selected,
/// and the default branch, first), then the tags loaded.
pub(super) fn from_rows(
    state: &State,
    repo: &RepoId,
    kind: RefKind,
    name: &str,
    q: &str,
) -> Vec<(
    crate::ui::overlays::PaletteItem,
    Option<crate::writes::Chosen>,
)> {
    let chosen = |from: &str| {
        Some(Chosen::RefFrom(
            repo.clone(),
            kind,
            name.to_owned(),
            from.to_owned(),
        ))
    };
    let q = q.trim();
    let branches = state.picked::<Results<BranchInfo>>(&DataKey::Branches(repo.clone()));
    let tags = state.picked::<Results<TagInfo>>(&DataKey::Tags(repo.clone()));
    let mut rows = Vec::new();
    let known = branches.is_some_and(|b| b.items.iter().any(|b| b.name == q))
        || tags.is_some_and(|t| t.items.iter().any(|t| t.name == q));
    if !q.is_empty() && !known {
        rows.push(item(
            format!("From {q}"),
            "a branch, a tag or a commit's SHA",
            chosen(q),
        ));
    }
    let Some(branches) = branches else {
        rows.push(item("Loading the branches…", "", None));
        return rows;
    };
    let here = picked(state, RefKind::Branch).map(|(n, _)| n);
    let mut hits: Vec<(usize, &BranchInfo)> = (branches.items.iter())
        .filter_map(|b| Some((fuzzy_score(q, &b.name)?, b)))
        .collect();
    hits.sort_by_key(|(score, b)| (Some(&b.name) != here.as_ref(), !b.default, *score));
    for (_, b) in hits {
        let hint = if b.default {
            "default branch"
        } else {
            "branch"
        };
        rows.push(item(b.name.clone(), hint, chosen(&b.name)));
    }
    for t in tags.iter().flat_map(|t| &t.items) {
        if fuzzy_score(q, &t.name).is_some() {
            rows.push(item(t.name.clone(), "tag", chosen(&t.name)));
        }
    }
    rows
}

/// Where it starts is chosen: in your clone, asks before pushing it;
/// elsewhere GitHub makes it, as its own dialog's button does.
#[must_use]
pub(super) fn from_chosen(
    state: &mut State,
    repo: RepoId,
    kind: RefKind,
    name: String,
    from: String,
) -> Vec<Cmd> {
    let clone = match clone_here(state, &repo) {
        Ok(clone) => clone,
        Err(cmds) => return cmds,
    };
    let subject = Subject::Ref {
        repo: repo.clone(),
        kind,
        name: name.clone(),
        from: Some(from.clone()),
    };
    match clone {
        Some(c) => {
            ask(
                state,
                Some(subject),
                format!("Create the {} {name} at {from}?", kind.noun()),
                vec![
                    (in_clone(&c), false),
                    (
                        format!("Your clone gets no {} of its own", kind.noun()),
                        false,
                    ),
                ],
                vec![("Create and push".into(), super::Answer::Push)],
            );
            Vec::new()
        }
        None => send(state, repo, kind, name, Some(from)),
    }
}

/// What a fact says of git doing it in your clone.
pub(super) fn in_clone(c: &Clone) -> String {
    let name = c.top.file_name().map_or_else(
        || c.top.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    format!("With git in your clone ({name}), pushed to {}", c.remote)
}

/// `,D` on the branches or tags: refused for the default branch and one
/// GitHub's rules protect; else asked.
#[must_use]
pub(super) fn ask_delete(state: &mut State, repo: &RepoId, kind: RefKind) -> Vec<Cmd> {
    if let Some(why) = refused(state, repo, what(kind)) {
        state.info(why);
        return Vec::new();
    }
    let Some((name, branch)) = picked(state, kind) else {
        state.info(format!("Select a {} first", kind.noun()));
        return Vec::new();
    };
    let pr = branch.and_then(|b| b.pr);
    let open_pr = pr.filter(|(_, s)| *s == crate::api::browse::PrState::Open);
    if branch.is_some_and(|b| b.default) {
        state.info(format!(
            "{name} is the default branch: GitHub doesn't delete it (another must be the default first, in the repository's settings)"
        ));
        return Vec::new();
    }
    let mut facts = Vec::new();
    if kind == RefKind::Branch {
        let key = DataKey::BranchRules(repo.clone(), name.clone());
        match state.answer::<BranchRules>(&key) {
            None => {
                let cmds = state.ensure(key, false);
                state.info(if cmds.is_empty() {
                    format!("Still asking GitHub whether {name} is protected")
                } else {
                    format!("Asking GitHub whether {name} is protected…")
                });
                state.housekeeping.deleting = Some((repo.clone(), name));
                return cmds;
            }
            Some(Err(why)) => {
                facts.push((
                    format!("GitHub didn't say whether it's protected: {why}"),
                    true,
                ));
            }
            Some(Ok(BranchRules {
                protected: true, ..
            })) => {
                state.info(format!(
                    "{name} is protected: GitHub's branch protection keeps it from being deleted"
                ));
                return Vec::new();
            }
            Some(Ok(BranchRules {
                undeletable: true, ..
            })) => {
                state.info(format!(
                    "{name} is protected: a ruleset of the repository's keeps it from being deleted"
                ));
                return Vec::new();
            }
            Some(Ok(_)) => {}
        }
    }
    let clone = match clone_here(state, repo) {
        Ok(clone) => clone,
        Err(cmds) => return cmds,
    };
    if let Some((n, _)) = open_pr {
        facts.push((
            format!("Its pull request #{n} is open: GitHub closes it"),
            true,
        ));
    }
    match &clone {
        Some(c) => {
            facts.push((in_clone(c), false));
            let noun = kind.noun();
            facts.push((format!("Your clone's own {noun} {name} stays"), false));
        }
        None if kind == RefKind::Tag => {
            facts.push((
                "A release of it, if it has one, becomes a draft".into(),
                false,
            ));
        }
        None => {
            if let Some((n, _)) = pr {
                facts.push((format!("GitHub can restore it from #{n}"), false));
            }
        }
    }
    ask(
        state,
        Some(Subject::Ref {
            repo: repo.clone(),
            kind,
            name: name.clone(),
            from: None,
        }),
        format!("Delete the {} {name}?", kind.noun()),
        facts,
        vec![(format!("Delete {}", kind.noun()), super::Answer::Delete)],
    );
    Vec::new()
}

/// Once GitHub says whether the branch asked about is protected: its
/// deletion is asked about, if it's still the one selected.
#[must_use]
pub(crate) fn follow(state: &mut State) -> Vec<Cmd> {
    let Some((repo, name)) = state.housekeeping.deleting.clone() else {
        return Vec::new();
    };
    let key = DataKey::BranchRules(repo.clone(), name.clone());
    if state.answer::<BranchRules>(&key).is_none() {
        return Vec::new();
    }
    state.housekeeping.deleting = None;
    let still = state.route() == Some(&Route::Code(code::Branches(repo.clone())))
        && picked(state, RefKind::Branch).is_some_and(|(n, _)| n == name)
        && state.overlay.is_none();
    if still {
        return ask_delete(state, &repo, RefKind::Branch);
    }
    Vec::new()
}

/// Makes or deletes it: with git in your clone when there's one, else
/// with GitHub's API.
#[must_use]
pub(super) fn send(
    state: &mut State,
    repo: RepoId,
    kind: RefKind,
    name: String,
    from: Option<String>,
) -> Vec<Cmd> {
    let clone = state.clone_of(&repo).flatten().cloned();
    let write = match from {
        Some(from) => RefWrite::Create(from),
        None => RefWrite::Delete,
    };
    crate::forms::sending(state);
    let doing = match (&write, &clone) {
        (RefWrite::Create(_), Some(_)) => "Pushing",
        (RefWrite::Create(_), None) => "Creating",
        (RefWrite::Delete, _) => "Deleting",
    };
    state.info(format!("{doing} the {} {name}…", kind.noun()));
    work::start(Save {
        repo,
        kind,
        name,
        write,
        clone,
    })
}

impl Job for Save {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        let Save {
            repo,
            kind,
            name,
            write,
            clone,
        } = self;
        let (gh, kind, git) = (&cx.gh, *kind, |e: String| Failure::msg(e));
        // GitHub names the commit, then git or GitHub's API makes the branch or
        // tag there.
        match (write, clone) {
            (RefWrite::Create(from), clone) => {
                let full_sha = from.len() == 40 && from.chars().all(|c| c.is_ascii_hexdigit());
                let sha = if full_sha {
                    from.clone()
                } else {
                    gh.commit_of(repo, from).await?
                };
                match clone {
                    Some(c) => (super::git::push_new_ref(&c.top, &c.remote, kind, name, &sha)
                        .await)
                        .map_err(git),
                    None => Ok(gh.create_ref(repo, kind, name, &sha).await?),
                }
            }
            (RefWrite::Delete, Some(c)) => {
                (super::git::push_delete(&c.top, &c.remote, kind, name).await).map_err(git)
            }
            (RefWrite::Delete, None) => Ok(gh.delete_ref(repo, kind, name).await?),
        }
    }

    /// GitHub's answer: the list follows it until it shows the change.
    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        let work = self;
        let Save {
            repo,
            kind,
            name,
            write,
            ..
        } = work;
        if let Err(why) = result {
            crate::forms::say(state, why.to_string());
            return Vec::new();
        }
        let page = match kind {
            RefKind::Branch => Route::Code(code::Branches(repo.clone())),
            RefKind::Tag => Route::Code(code::Tags(repo.clone())),
        };
        crate::forms::close_sender(state, &page);
        let there = matches!(write, RefWrite::Create(_));
        // A branch made again with its name is asked about again.
        (state.data).remove(&DataKey::BranchRules(repo.clone(), name.clone()));
        state.info(format!(
            "{} the {} {name}",
            if there { "Created" } else { "Deleted" },
            kind.noun()
        ));
        if state.route() != Some(&page) {
            return Vec::new();
        }
        expect(state, page.clone(), Expected::Ref(repo, kind, name, there));
        state.ensure_route(&page, true)
    }
}

/// The menu's rows on the branches and tags.
pub(super) fn doables(state: &State, route: &Route, out: &mut Vec<Doable>) {
    let (repo, kind) = match route {
        Route::Code(code::Branches(repo)) => (repo, RefKind::Branch),
        Route::Code(code::Tags(repo)) => (repo, RefKind::Tag),
        _ => return,
    };
    let why = refused(state, repo, what(kind));
    let noun = kind.noun();
    out.push(row(
        "New",
        Action::NewHere,
        format!("New {noun}"),
        "new",
        why.clone(),
    ));
    let picked = picked(state, kind);
    let row_why = why.or_else(|| match &picked {
        None => Some(format!("no {noun} is selected")),
        Some((name, Some(b))) if b.default => Some(format!("{name} is the default branch")),
        Some(_) => None,
    });
    let name = picked.map_or_else(|| format!("the {noun}"), |(n, _)| n);
    out.push(row(
        "Edit",
        Action::DeleteComment,
        format!("Delete {name}"),
        "delete",
        row_why,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_gits() {
        assert_eq!(name_problem("feature/x"), None);
        assert_eq!(name_problem("v1.2.0"), None);
        assert!(name_problem("has space").is_some());
        assert!(name_problem("a..b").is_some());
        assert!(name_problem("-x").is_some());
        assert!(name_problem("x.lock").is_some());
        assert!(name_problem("a/.b").is_some());
    }
}
