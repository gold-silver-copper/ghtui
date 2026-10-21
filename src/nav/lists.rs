//! Lists: their state and sort.

use super::*;
use crate::route::{people, runs, search, security};

/// The list query with its state (`is:open`, `is:unread`…) replaced by
/// `state`.
pub(super) fn with_state(query: &str, state: &str) -> String {
    let mut words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| {
            !matches!(
                *w,
                "is:open" | "is:closed" | "state:open" | "state:closed" | "is:unread" | "is:read"
            )
        })
        .collect();
    let qualifier = match state {
        "open" => Some("is:open"),
        "closed" => Some("is:closed"),
        "unread" => Some("is:unread"),
        "read" => Some("is:read"),
        _ => None,
    };
    if let Some(q) = qualifier {
        words.insert(0, q);
    }
    words.join(" ")
}

/// Open → closed → all; in the inbox, unread → read → all.
pub(super) fn next_list_state(route: &Route, query: &str) -> &'static str {
    match pages::list_state(query) {
        "open" => "closed",
        "unread" => "read",
        "closed" | "read" => "all",
        _ if matches!(route, Route::Search(search::Notifications { .. })) => "unread",
        _ => "open",
    }
}

/// The list on screen and its filter.
pub(super) fn list_query(state: &State) -> Option<(Route, String)> {
    let route = state.route()?;
    Some((route.clone(), route.family().query()?.to_owned()))
}

pub(super) fn set_list_state(state: &mut State, which: &str) -> Cmds {
    let Some((route, query)) = list_query(state) else { return Cmds::new() };
    match route.family().with_query(with_state(&query, which)) {
        Some(r) if r != route => state.replace(r, true, None),
        _ => Cmds::new(),
    }
}

/// What a search finds that's neither open nor closed, nor sorted as
/// issues are: repositories, people, commits, code.
pub(super) fn plain_search(route: &Route) -> Option<&'static str> {
    use crate::api::browse::SearchKind as K;
    match route {
        Route::Search(search::Search { kind, .. }) => match kind {
            K::Repos => Some("Repositories"),
            K::Users => Some("People"),
            K::Commits => Some("Commits"),
            K::Code => Some("Code results"),
            K::Issues | K::Pulls | K::Discussions => None,
        },
        _ => None,
    }
}

pub(super) fn cycle_state(state: &mut State) -> Cmds {
    if let Some(what) = state.route().and_then(plain_search) {
        return state.just_say(format!("{what} aren't open or closed"));
    }
    if let Some(Route::Runs(runs::Actions { .. })) = state.route() {
        let filter = state.first_key(Action::Search);
        return state.just_say(format!(
            "Runs aren't open or closed · {filter} filters them: is:failure, is:success…"
        ));
    }
    let Some((route, query)) = list_query(state) else {
        return state.just_say("Only lists have open and closed");
    };
    set_list_state(state, next_list_state(&route, &query))
}

pub(super) fn cycle_sort(state: &mut State) -> Cmds {
    // A profile's repositories sort by GraphQL order, not a search word.
    if let Some(Route::People(people::User {
        login,
        tab: ProfileTab::Repositories(sort),
    })) = state.route()
    {
        let next = sort.next();
        let route = Route::People(people::User {
            login: login.clone(),
            tab: ProfileTab::Repositories(next),
        });
        state.info(format!("Sorted: {}", next.label()));
        return state.replace(route, true, None);
    }
    if let Some(Route::Runs(runs::Actions { .. })) = state.route() {
        return state.just_say("Runs are newest first: GitHub doesn't sort them otherwise");
    }
    if let Some(Route::Search(search::Notifications { .. })) = state.route() {
        return state
            .just_say("Notifications are newest first: GitHub doesn't sort them otherwise");
    }
    if let Some(Route::Security(security::Alerts { .. })) = state.route() {
        return state.just_say("Alerts are newest first: ghtui doesn't sort them otherwise");
    }
    if let Some(what) = state.route().and_then(plain_search) {
        return state.just_say(format!("{what} come in GitHub's best-match order"));
    }
    let Some((route, query)) = list_query(state) else {
        return state.just_say("Only lists can be sorted");
    };
    let current = pages::sort_label(&query);
    let i = pages::SORTS
        .iter()
        .position(|(_, l)| *l == current)
        .unwrap_or(0);
    let Some(&(next, label)) = pages::SORTS.iter().cycle().nth(i + 1) else {
        return Cmds::new();
    };
    let mut words: Vec<&str> = query
        .split_whitespace()
        .filter(|w| !w.starts_with("sort:"))
        .collect();
    let sort = format!("sort:{next}");
    if next != "created-desc" {
        words.push(&sort);
    }
    state.info(format!("Sorted: {label}"));
    match route.family().with_query(words.join(" ")) {
        Some(r) => state.replace(r, true, None),
        None => Cmds::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_states_rewrite_the_query() {
        assert_eq!(
            with_state("is:open label:bug", "closed"),
            "is:closed label:bug"
        );
        assert_eq!(with_state("is:closed", "all"), "");
        assert_eq!(with_state("label:bug", "open"), "is:open label:bug");
        assert_eq!(
            with_state("is:unread reason:mention", "read"),
            "is:read reason:mention"
        );
    }
}
