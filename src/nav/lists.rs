//! Lists: their state and sort.

use super::*;

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
        _ if matches!(route, Route::Notifications { .. }) => "unread",
        _ => "open",
    }
}

/// The list on screen and its filter.
pub(super) fn list_query(state: &State) -> Option<(Route, String)> {
    let route = state.route()?;
    Some((route.clone(), route.list_query()?.to_owned()))
}

#[must_use]
pub(super) fn set_list_state(state: &mut State, which: &str) -> Vec<Cmd> {
    let Some((route, query)) = list_query(state) else {
        return Vec::new();
    };
    match route.with_query(with_state(&query, which)) {
        Some(r) if r != route => state.replace(r, true, None),
        _ => Vec::new(),
    }
}

#[must_use]
pub(super) fn cycle_state(state: &mut State) -> Vec<Cmd> {
    if let Some(Route::Actions { .. }) = state.route() {
        let filter = state.first_key(Action::Search);
        state.info(format!(
            "Runs aren't open or closed · {filter} filters them: is:failure, is:success…"
        ));
        return Vec::new();
    }
    let Some((route, query)) = list_query(state) else {
        state.info("Only lists have open and closed");
        return Vec::new();
    };
    set_list_state(state, next_list_state(&route, &query))
}

#[must_use]
pub(super) fn cycle_sort(state: &mut State) -> Vec<Cmd> {
    // A profile's repositories sort by GraphQL order, not a search word.
    if let Some(Route::User {
        login,
        tab: ProfileTab::Repositories(sort),
    }) = state.route()
    {
        let next = sort.next();
        let route = Route::User {
            login: login.clone(),
            tab: ProfileTab::Repositories(next),
        };
        state.info(format!("Sorted: {}", next.label()));
        return state.replace(route, true, None);
    }
    if let Some(Route::Actions { .. }) = state.route() {
        state.info("Runs are newest first: GitHub doesn't sort them otherwise");
        return Vec::new();
    }
    if let Some(Route::Notifications { .. }) = state.route() {
        state.info("Notifications are newest first: GitHub doesn't sort them otherwise");
        return Vec::new();
    }
    if let Some(Route::Alerts { .. }) = state.route() {
        state.info("Alerts are newest first: ghtui doesn't sort them otherwise");
        return Vec::new();
    }
    let Some((route, query)) = list_query(state) else {
        state.info("Only lists can be sorted");
        return Vec::new();
    };
    let current = pages::sort_label(&query);
    let i = pages::SORTS
        .iter()
        .position(|(_, l)| *l == current)
        .unwrap_or(0);
    let Some(&(next, label)) = pages::SORTS.iter().cycle().nth(i + 1) else {
        return Vec::new();
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
    match route.with_query(words.join(" ")) {
        Some(r) => state.replace(r, true, None),
        None => Vec::new(),
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
