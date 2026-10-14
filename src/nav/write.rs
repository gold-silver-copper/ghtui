//! Writing: star and comment.

use super::*;
use crate::route::code;

#[must_use]
pub fn star(state: &mut State) -> Vec<Cmd> {
    let Some(repo) = state.route().and_then(Route::repo).cloned() else {
        state.info("Open a repository to star it");
        return Vec::new();
    };
    let Some(overview) = state.overview(&repo) else {
        state.info("The repository hasn't loaded yet");
        return Vec::new();
    };
    let (id, starred) = (overview.id.clone(), !overview.starred);
    set_starred(state, &repo, starred);
    let star = Change::Star { repo, id, starred };
    vec![Cmd::Api(Api::Change(star, crate::act::By::Star))]
}

/// Shows a repository as starred or not (optimistically, before GitHub
/// confirms).
pub fn set_starred(state: &mut State, repo: &RepoId, starred: bool) {
    if let Some(Remote {
        data: Some(Data::Repo(o)),
        ..
    }) = state.data.get_mut(&DataKey::Repo(repo.clone()))
        && o.starred != starred
    {
        o.starred = starred;
        o.summary.stars = if starred {
            o.summary.stars + 1
        } else {
            o.summary.stars.saturating_sub(1)
        };
    }
    state.data_gen += 1;
}

/// `>`: a reply quoting the selected comment, under it.
#[must_use]
pub(super) fn quote_reply(state: &mut State) -> Vec<Cmd> {
    let Screen::Page(p) = state.screen() else {
        return Vec::new();
    };
    match p.selected_link().cloned() {
        Some(Link::Quote { author, body, at }) => {
            let quoted: String = body.trim().lines().map(|l| format!("> {l}\n")).collect();
            comment_with(state, &format!("> @{author} wrote:\n{quoted}\n"), Some(at))
        }
        _ => {
            let down = state.first_key(Action::NextItem);
            state.info(format!(
                "Select a comment to quote: {down} goes comment by comment"
            ));
            Vec::new()
        }
    }
}

/// Opens the composer on the issue or pull request on screen, starting
/// with `text` (quoting the comment made `quoting`).
#[must_use]
pub(super) fn comment_with(state: &mut State, text: &str, quoting: Option<String>) -> Vec<Cmd> {
    let target = match state.route() {
        Some(Route::Issue { repo, number }) => {
            let key = DataKey::Issue(repo.clone(), *number);
            match state.get(&key) {
                Some(Data::Issue(Some(issue))) => {
                    Some((issue.id.clone(), format!("{repo}#{number}")))
                }
                _ => None,
            }
        }
        Some(Route::Pr { pr, .. }) => state.activity(pr).map(|a| (a.id.clone(), pr.to_string())),
        // A new issue's or pull request's title and description.
        Some(Route::NewIssue(_) | Route::NewPr(_)) => return crate::create::compose(state),
        Some(Route::Discussion { .. }) => return crate::discuss::act(state, Action::Comment),
        Some(route) if crate::writes::is_form(route) => return crate::writes::compose(state),
        // A commit's comments.
        Some(Route::Code(code::Commit { repo, oid, .. })) => {
            let target = ComposeTarget::Commit {
                repo: repo.clone(),
                oid: oid.clone(),
            };
            return state.compose(target, text);
        }
        _ => {
            state.info("Comments go on issues and pull requests");
            return Vec::new();
        }
    };
    let Some((subject_id, name)) = target else {
        state.info("Still loading");
        return Vec::new();
    };
    let target = ComposeTarget::Conversation {
        subject_id,
        name,
        quoting,
    };
    state.compose(target, text)
}
