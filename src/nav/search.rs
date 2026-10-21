//! The search box, with its suggestions.

use super::*;
use crate::route::{code, people, runs, search, security};
use std::borrow::Cow;

pub struct SearchBox {
    pub input: TextArea<'static>,
    /// Filtering the list on screen rather than searching GitHub.
    pub filter: bool,
    pub selected: usize,
}

impl SearchBox {
    /// What's typed, trimmed.
    pub(crate) fn query(&self) -> String {
        self.input.lines().join("").trim().to_owned()
    }
}

/// What choosing a suggestion does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    Go(Dest),
    Search(SearchKind, String),
    Filter(String),
}

/// A key typed into a one-line box. Ctrl-u clears what's before the
/// cursor, as in a shell, rather than undoing.
pub fn type_into(input: &mut TextArea<'_>, key: KeyEvent) {
    if key.code == KeyCode::Char('u') && key.modifiers.contains(KeyModifiers::CONTROL) {
        input.delete_line_by_head();
    } else {
        input.input(key);
    }
}

pub fn new_input(theme: &Theme, bg: Bg) -> TextArea<'static> {
    let mut input = TextArea::default();
    input.set_style(theme.body(bg));
    input.set_cursor_line_style(theme.body(bg));
    input.set_cursor_style(theme.fill(Bg::Primary));
    input
}

impl State {
    /// The header's search box: searching GitHub (`github`), or the
    /// page's own search: a list's filter, a job's log.
    pub fn open_search(&mut self, github: bool) -> Cmds {
        let filter_query = match self.route() {
            _ if github => None,
            Some(Route::Runs(runs::Job { query, .. })) => Some(query.clone()),
            route => route.and_then(|r| r.family().query()).map(str::to_owned),
        };
        let mut input = new_input(&self.theme, Bg::ContainerHighest);
        if let Some(q) = &filter_query {
            input.insert_str(q);
            input.insert_char(' ');
        }
        // A profile's repositories filter by searching that owner's.
        if !github
            && let Some(Route::People(people::User {
                login,
                tab: ProfileTab::Repositories(_),
            })) = self.route()
        {
            input.insert_str(format!("user:{login} "));
        }
        // A filter is edited in place, on the page's own field: scrolled
        // into view.
        if filter_query.is_some()
            && let Screen::Page(p) = self.screen_mut()
            && let Some(line) = p.page.line_of(&Link::Filter)
        {
            p.scroll = p.scroll.min(line);
        }
        self.overlay = Some(Overlay::Search(Box::new(SearchBox {
            input,
            filter: filter_query.is_some(),
            selected: 0,
        })));
        Cmds::new()
    }

    /// Where the search box is typed into: the page's own filter field
    /// when it filters and the field is in view, else the header's.
    pub fn search_field(&self, sb: &SearchBox) -> (Rect, bool) {
        match self.filter_field().filter(|_| sb.filter) {
            Some(field) => (field, true),
            None => (self.header_search(), false),
        }
    }

    /// The page's filter field (`⌕ is:open`), while it's in view.
    pub(super) fn filter_field(&self) -> Option<Rect> {
        let Screen::Page(p) = self.screen() else {
            return None;
        };
        let area = self.page_area();
        let row = p.page.line_of(&Link::Filter)?.checked_sub(p.scroll)?;
        let y = area.y.checked_add(u16::try_from(row).ok()?)?;
        let x = page::columns(&p.page, area).main_x;
        let width = p.page.width.min(area.right().saturating_sub(x));
        (y < area.bottom()).then_some(Rect::new(x, y, width, 1))
    }

    /// The header's search field.
    pub fn header_search(&self) -> Rect {
        let chrome = self.chrome();
        let crumbs: Vec<_> = chrome.crumbs.iter().map(|(c, _)| c.clone()).collect();
        let right: Vec<_> = chrome.right.iter().map(|(t, _)| t.clone()).collect();
        chrome::header_layout(self.layout().header, &crumbs, &right).search
    }

    /// "→ o/r#5 go to", for what's typed when it's a place or ends in a
    /// link.
    fn go_to(&self, q: &str) -> Option<(SuggestRow, Option<Pick>)> {
        let dest = link_in(q, self.context_repo())?;
        let label = match &dest.target {
            Target::Page(r) => r.title(),
            Target::Files(of) => format!("Files changed in {of}"),
            Target::External(url) => url.clone(),
        };
        Some(suggestion("→", label, "go to", "↵", Pick::Go(dest)))
    }

    /// The search box's suggestions: rows to draw, and what choosing each
    /// item does (headings have none).
    pub fn suggestions(&self, sb: &SearchBox) -> Vec<(SuggestRow, Option<Pick>)> {
        let q: &str = &sb.query();
        let mut out = Vec::new();
        if sb.filter {
            // A job's log filters by its text alone.
            if let Some(Route::Runs(runs::Job { .. })) = self.route() {
                let pick = Pick::Filter(q.to_owned());
                let what = if q.is_empty() {
                    "clear the search"
                } else {
                    "search the log"
                };
                out.push(suggestion("⌕", q, what, "↵", pick));
                return out;
            }
            // A link typed or pasted after the filter goes there.
            if let Some(go) = self.go_to(q) {
                out.push(go);
            }
            if !q.is_empty() {
                let pick = Pick::Filter(q.to_owned());
                out.push(suggestion("⌕", q, "filter this list", "↵", pick));
            }
            out.push(heading("Quick filters"));
            // Alerts and runs filter by their own words (code scanning and
            // runs on the default branch too), and aren't searched; nor
            // are notifications.
            let (on, filters, searched) = match self.route() {
                Some(Route::Security(security::Alerts { repo, kind, .. })) => {
                    let scans = *kind == crate::api::security::AlertKind::CodeScanning;
                    (scans.then_some(repo), kind.filters(), false)
                }
                Some(Route::Runs(runs::Actions { repo, .. })) => (Some(repo), RUN_FILTERS, false),
                Some(Route::Search(search::Notifications { .. })) => (None, INBOX_FILTERS, false),
                _ => (None, ISSUE_FILTERS, true),
            };
            let overview = on.and_then(|repo| self.overview(repo));
            if let Some(b) = overview.and_then(|o| o.default_branch.as_ref()) {
                let f = format!("branch:{b}");
                let pick = Pick::Filter(f.clone());
                out.push(suggestion("⚑", f, &format!("on {b}"), "", pick));
            }
            for (f, what) in filters {
                out.push(suggestion("⚑", *f, what, "", Pick::Filter((*f).to_owned())));
            }
            if searched && !q.is_empty() {
                out.push(heading("Search all of GitHub"));
                let pick = Pick::Search(SearchKind::Issues, q.to_owned());
                out.push(suggestion("⌕", q, "issues and pull requests", "", pick));
            }
            return out;
        }
        if let Some(go) = self.go_to(q) {
            out.push(go);
        }
        // Jump to: pages you've visited, your repositories, live matches.
        let current = self.route().map(Route::url);
        let now = (self.clock)();
        // Only the visits that match are sorted, and only the rows shown
        // are copied.
        let mut visits: Vec<(usize, &Visit)> = (self.visits.iter())
            .filter(|v| Some(&v.url) != current.as_ref())
            .filter_map(|v| Some((fuzzy_score(q, &v.title)?, v)))
            .collect();
        visits.sort_by_key(|(_, v)| std::cmp::Reverse(v.score(now)));
        let mut jumps: Vec<Jump> = (visits.into_iter())
            .map(|(score, v)| {
                (
                    score,
                    v.title.as_str().into(),
                    v.url.as_str().into(),
                    "◷",
                    "visited",
                )
            })
            .collect();
        for r in self.home_repos() {
            let name = r.repo.to_string();
            if let Some(score) = fuzzy_score(q, &name) {
                let url = pages::url::repo(&r.repo);
                jumps.push((score + 1, name.into(), url.into(), "▤", "on your Home"));
            }
        }
        // Most recent first, as they came, until something is typed.
        if !q.is_empty() {
            jumps.sort_by_key(|(score, ..)| *score);
        }
        let most = if q.is_empty() { 8 } else { 5 };
        let mut seen: Vec<String> = Vec::new();
        // A page whose title has what's typed comes before searching for
        // it; one that only has its letters scattered, after.
        let (near, far): (Vec<_>, Vec<_>) =
            (jumps.into_iter()).partition(|(score, ..)| q.is_empty() || *score < 1000);
        let mut jump = |out: &mut Vec<_>, jumps: Vec<Jump>| {
            let mut headed = false;
            for (_, title, url, icon, detail) in jumps {
                if seen.len() >= most {
                    break;
                }
                if seen.iter().any(|s| *s == url) {
                    continue;
                }
                if !headed {
                    out.push(heading(if q.is_empty() { "Recent" } else { "Jump to" }));
                    headed = true;
                }
                let pick = Pick::Go(Dest::from_url(&url));
                out.push(suggestion(icon, title, detail, "", pick));
                seen.push(url.into_owned());
            }
        };
        jump(&mut out, near);
        if !q.is_empty() {
            out.push(heading("Search"));
            if let Some(repo) = self.context_repo().filter(|_| !q.starts_with("repo:")) {
                let pick = Pick::Search(SearchKind::Issues, format!("repo:{repo} {q}"));
                out.push(suggestion("⌕", q, &format!("in {repo}"), "", pick));
            }
            for (kind, what) in [
                (SearchKind::Repos, "repositories"),
                (SearchKind::Issues, "issues"),
                (SearchKind::Pulls, "pull requests"),
                (SearchKind::Users, "users"),
            ] {
                let pick = Pick::Search(kind, q.to_owned());
                out.push(suggestion("⌕", q, what, "", pick));
            }
        }
        jump(&mut out, far);
        // Live matches come last, so they never move what's selected.
        let suggested = self.picked::<[RepoSummary]>(&DataKey::Suggest(q.to_owned()));
        if let Some(suggested) = suggested.filter(|_| !q.is_empty()) {
            let live: Vec<&RepoSummary> = suggested
                .iter()
                .filter(|r| !seen.contains(&pages::url::repo(&r.repo)))
                .collect();
            if !live.is_empty() {
                out.push(heading("Repositories"));
            }
            for r in live {
                let detail = format!("☆ {}", pages::compact(r.stars));
                let pick = Pick::Go(Target::Page(Route::Code(code::Repo(r.repo.clone()))).into());
                out.push(suggestion("▤", r.repo.to_string(), &detail, "", pick));
            }
        }
        out
    }
}

/// Where what's typed goes, if it is a place or ends in a link: all of
/// it (`o/r#5`, `@octocat`), or its last word when that's a URL or a
/// GitHub path, typed after a filter or a search's `repo:o/r`.
pub(super) fn link_in(q: &str, context: Option<&RepoId>) -> Option<Dest> {
    if let Some(dest) = route::parse_input(q, context) {
        return Some(dest);
    }
    let last = q.split_whitespace().last()?;
    let link = last.contains("://")
        || last.matches('/').count() >= 2
        || (last.contains('/') && last.contains('#'));
    link.then(|| route::parse_input(last, context)).flatten()
}

/// A page to jump to: how well it matches, its title and link, its icon
/// and why it's offered.
type Jump<'a> = (
    usize,
    Cow<'a, str>,
    Cow<'a, str>,
    &'static str,
    &'static str,
);

/// GitHub Actions' words for a repository's runs.
const RUN_FILTERS: &[(&str, &str)] = &[
    ("is:failure", "failed"),
    ("is:in_progress", "running"),
    ("is:queued", "queued"),
    ("is:success", "succeeded"),
    ("event:pull_request", "for pull requests"),
    ("event:push", "for pushes"),
];

/// Why notifications came.
const INBOX_FILTERS: &[(&str, &str)] = &[
    ("is:unread", "unread"),
    ("reason:review-requested", "review requested"),
    ("reason:mention", "mentioning you"),
    ("reason:assign", "assigned to you"),
    ("reason:participating", "participating"),
    ("reason:team-mention", "mentioning your team"),
    ("reason:ci-activity", "CI activity"),
];

/// Issues' and pull requests' common searches.
const ISSUE_FILTERS: &[(&str, &str)] = &[
    ("is:open", "open"),
    ("is:closed", "closed"),
    ("is:open author:@me", "yours"),
    ("is:open assignee:@me", "assigned to you"),
    ("is:open mentions:@me", "mentioning you"),
    ("is:open no:assignee", "unassigned"),
];

/// A suggestion to choose.
pub(super) fn suggestion(
    icon: &str,
    label: impl Into<String>,
    detail: &str,
    hint: &str,
    pick: Pick,
) -> (SuggestRow, Option<Pick>) {
    let row = SuggestRow::Item {
        icon: icon.to_owned(),
        label: label.into(),
        detail: detail.to_owned(),
        hint: hint.to_owned(),
    };
    (row, Some(pick))
}

pub(super) fn heading(text: &str) -> (SuggestRow, Option<Pick>) {
    (SuggestRow::Heading(text.to_owned()), None)
}

pub fn on_search_box_key(state: &mut State, key: KeyEvent) -> Cmds {
    let Some(Overlay::Search(sb)) = &state.overlay else {
        return Cmds::new();
    };
    let picks: Vec<Pick> = state
        .suggestions(sb)
        .into_iter()
        .filter_map(|(_, p)| p)
        .collect();
    let Some(Overlay::Search(sb)) = &mut state.overlay else {
        return Cmds::new();
    };
    if move_in_list(key, &mut sb.selected, picks.len()) {
        return Cmds::new();
    }
    match key.code {
        KeyCode::Esc => {
            state.overlay = None;
            Cmds::new()
        }
        KeyCode::Enter => {
            let pick = picks.into_iter().nth(sb.selected);
            state.overlay = None;
            pick.map_or_else(Cmds::new, |p| choose(state, p))
        }
        _ => {
            type_into(&mut sb.input, key);
            search_box_changed(sb)
        }
    }
}

/// After the search box's text changed: its suggestions start over, and
/// GitHub is asked for some once typing pauses.
pub fn search_box_changed(sb: &mut SearchBox) -> Cmds {
    sb.selected = 0;
    let q = sb.query();
    if !sb.filter && q.chars().count() >= 2 && link_in(&q, None).is_none() {
        Cmd::SuggestLater(q).into()
    } else {
        Cmds::new()
    }
}

pub(super) fn choose(state: &mut State, pick: Pick) -> Cmds {
    match pick {
        Pick::Go(dest) => state.go(dest),
        Pick::Search(kind, query) => state.push(Route::Search(search::Search { kind, query })),
        Pick::Filter(query) if matches!(state.route(), Some(Route::Runs(runs::Job { .. }))) => {
            search_log(state, &query);
            Cmds::new()
        }
        Pick::Filter(query) => match state
            .route()
            .and_then(|r| r.family().with_query(query.clone()))
        {
            Some(route) => state.replace(route, true, None),
            None => state.push(Route::Search(search::Search {
                kind: SearchKind::Issues,
                query,
            })),
        },
    }
}

/// Searches the job's log on screen for `query` (empty clears it) and
/// says how it went. The page stays as it is: the same place in history,
/// the step a link pointed at, nothing fetched again.
pub fn search_log(state: &mut State, query: &str) {
    let Screen::Page(p) = state.screen_mut() else {
        return;
    };
    let Route::Runs(runs::Job {
        repo,
        job,
        query: q,
        ..
    }) = &mut p.route
    else {
        return;
    };
    let log = DataKey::JobLog(repo.clone(), *job);
    query.clone_into(q);
    // Rebuilt, it opens at the first match, or where it opened before.
    (p.built, p.mark, p.jumped) = (None, None, None);
    state.sync_page();
    let Screen::Page(p) = state.screen() else {
        return;
    };
    let loaded = state
        .picked::<crate::api::browse::JobLog>(&log)
        .is_some_and(|l| !l.running);
    let text = match (p.mark_at(), p.page.marks.len()) {
        _ if query.is_empty() => "Cleared the search".to_owned(),
        (Some(at), of) => format!("Match {} of {of}", at + 1),
        _ if loaded => format!("No matches for “{query}”"),
        _ => format!("Searches for “{query}” once the log is in"),
    };
    state.info(text);
}
