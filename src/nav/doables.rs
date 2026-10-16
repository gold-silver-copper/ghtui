//! What you can do here: the actions menu's rows and the keys hinted in
//! the status bar.

use super::*;
use crate::route::{code, issues, pulls, runs, search};

/// Something you can do here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doable {
    pub section: &'static str,
    pub action: Action,
    /// For the menu.
    pub label: String,
    /// For the status bar; empty keeps it out.
    pub short: &'static str,
    pub unavailable: Option<String>,
}

/// A menu row.
pub(crate) fn row(
    section: &'static str,
    action: Action,
    label: impl Into<String>,
    short: &'static str,
    unavailable: Option<String>,
) -> Doable {
    Doable {
        section,
        action,
        label: label.into(),
        short,
        unavailable,
    }
}

/// The diff's menu, section by section, with the status bar's few.
pub(super) const DIFF_DOABLES: &[(&str, &[(Action, &str)])] = {
    use Action as A;
    &[
        (
            "Move",
            &[
                (A::NextHunk, "change"),
                (A::PrevHunk, ""),
                (A::NextFile, "file"),
                (A::PrevFile, ""),
                (A::NextUnviewed, ""),
                (A::NextThread, ""),
                (A::PrevThread, ""),
                (A::JumpMove, ""),
                (A::OldSide, ""),
                (A::NewSide, ""),
                (A::Search, ""),
                (A::SearchNext, ""),
                (A::SearchPrev, ""),
                (A::SearchWord, ""),
                (A::WordNext, ""),
                (A::WordPrev, ""),
                (A::Symbols, ""),
                (A::Definition, ""),
                (A::RepoSymbols, ""),
                (A::References, ""),
                (A::FindFile, ""),
                (A::SwitchPane, ""),
                (A::Fold, ""),
                (A::Unfold, ""),
            ],
        ),
        (
            "View",
            &[
                (A::FoldToggle, ""),
                (A::FoldAll, ""),
                (A::UnfoldAll, ""),
                (A::ToggleTree, ""),
                (A::TreeNarrower, ""),
                (A::TreeWider, ""),
                (A::ToggleSplit, ""),
                (A::IgnoreWhitespace, ""),
                (A::BySyntax, ""),
                (A::ExpandContext, ""),
                (A::FullFile, ""),
                (A::ToggleSinceReview, ""),
                (A::PickCommits, ""),
            ],
        ),
        (
            "Review",
            &[
                (A::ToggleViewed, "viewed"),
                (A::MarkReviewed, ""),
                (A::Comment, "comment"),
                (A::VisualLines, ""),
                (A::Suggest, ""),
                (A::FileComment, ""),
                (A::ResolveThread, ""),
                (A::Delete, ""),
                (A::UndoDelete, ""),
                (A::SubmitReview, "submit review"),
            ],
        ),
    ]
};

impl State {
    /// What you can do on this screen, in sections, most useful first.
    pub fn doables(&self) -> Vec<Doable> {
        let mut out = Vec::new();
        let selected = match self.screen() {
            Screen::Page(p) => {
                self.page_doables(p, &mut out);
                p.selected_link()
            }
            Screen::Diff(_) => {
                for (section, actions) in DIFF_DOABLES {
                    for &(action, short) in *actions {
                        out.push(row(section, action, action.description(), short, None));
                    }
                }
                None
            }
        };
        crate::features::offers(self, &mut out);
        let go = |action, label: &str, short| row("Go", action, label, short, None);
        out.push(go(
            Action::SearchGithub,
            "Search GitHub or go to a page",
            "",
        ));
        if !self.chrome().tabs.is_empty() {
            out.push(go(Action::NextTab, "Next tab", "tabs"));
            out.push(go(Action::PrevTab, "Previous tab", ""));
        }
        if let Screen::Diff(_) = self.screen() {
            out.push(go(Action::Tab1, "Back to the conversation", ""));
        }
        if let Some(n) = self.review_tab() {
            let mut glance = go(
                Action::ReviewGlance,
                "The review at a glance: drafts, threads, reviewers",
                "",
            );
            if self.chrome().active == Some(n - 1) {
                glance.unavailable = Some("this is it".into());
            }
            out.push(glance);
        }
        let (open, copy) = match (selected, self.screen()) {
            (Some(_), _) => ("Open the selection on GitHub", "Copy the selection's link"),
            (None, Screen::Diff(_)) => (
                "Open the lines or file on GitHub",
                "Copy the link to the lines or file",
            ),
            (None, Screen::Page(_)) => ("Open this page on GitHub", "Copy this page's link"),
        };
        out.push(go(Action::OpenInBrowser, open, "browser"));
        out.push(go(Action::Copy, copy, "copy link"));
        let mut link = go(Action::OpenLink, Action::OpenLink.description(), "");
        if self.link_under_cursor().is_none() {
            link.unavailable = Some("no link is under the cursor".into());
        }
        out.push(link);
        out.push(go(Action::Hints, "Follow any link by its letters", "links"));
        if let Screen::Page(_) = self.screen() {
            out.push(go(Action::UpLevel, "Up a level", ""));
        }
        let mut back = go(Action::Back, "Back", "");
        if self.screens.len() < 2 {
            back.unavailable = Some("this is the first page".into());
        }
        out.push(back);
        let mut fwd = go(Action::Forward, "Forward", "");
        if self.forward.is_empty() {
            fwd.unavailable = Some("nothing to go forward to".into());
        }
        out.push(fwd);
        out.push(go(Action::GoHome, "Home", ""));
        out.push(go(Action::Notifications, "Notifications", ""));
        out.push(go(
            Action::Explore,
            "Explore: new repositories and topics",
            "",
        ));
        out.push(go(Action::Refresh, "Refresh", ""));
        out.push(go(Action::CommandPalette, "Command palette", ""));
        let mut messages = go(Action::Messages, "Recent messages and errors", "");
        if self.notices.log().is_empty() {
            messages.unavailable = Some("nothing yet".into());
        }
        out.push(messages);
        let tab = |action, label: &str| row("Tabs", action, label, "", None);
        out.push(tab(Action::OpenInTab, "Open in a new tab"));
        if self.tab_count() > 1 {
            out.push(tab(Action::NextOpenTab, "Next open tab"));
            out.push(tab(Action::PrevOpenTab, "Previous open tab"));
            out.push(tab(Action::CloseTab, "Close the tab"));
        }
        let contexts = self.contexts();
        for action in [
            Action::Down,
            Action::Up,
            Action::NextSection,
            Action::PrevSection,
            Action::Top,
            Action::Bottom,
            Action::PageDown,
            Action::PageUp,
            Action::ScrollLeft,
            Action::ScrollRight,
            Action::Cancel,
            Action::Quit,
        ] {
            if contexts.contains(&action.context()) || self.key_here(action).is_some() {
                out.push(row("Keys", action, action.description(), "", None));
            }
        }
        out
    }

    pub(super) fn page_doables(&self, p: &PageScreen, out: &mut Vec<Doable>) {
        let here = |action, label: &str, short| row("This page", action, label, short, None);
        let route = &p.route;
        // A file's line is this page, as a log's is: nothing to open.
        let line = matches!(route, Route::Runs(runs::Job { .. })) && p.code_line().is_some();
        if !matches!(route, Route::Code(code::Blob { .. })) && !line {
            let mut open = here(Action::Open, "Open the selected row", "open");
            match p.selected_link() {
                Some(link) => {
                    open.label = describe(link);
                    // An artifact or an asset is saved, not opened.
                    if matches!(link, Link::Artifact { .. } | Link::Asset(_)) {
                        open.short = "save";
                    }
                }
                None => open.unavailable = Some("nothing is selected".into()),
            }
            out.push(open);
        }
        let list = route.search().map(|(_, q)| q);
        // `/` searches the page, or filters a list; then n/N.
        let searched = match route {
            Route::Runs(runs::Job { query, .. }) => {
                out.push(here(Action::Search, "Search the log", "search"));
                Some(query.clone())
            }
            _ if route.family().query().is_some() => {
                out.push(here(Action::Search, "Filter this list", "filter"));
                None
            }
            _ => {
                out.push(here(Action::Search, "Search this page", "search"));
                p.find.clone()
            }
        };
        if let Some(query) = searched {
            let why = if query.is_empty() {
                Some("nothing is searched yet".to_owned())
            } else if p.page.marks.is_empty() {
                Some(format!("no line has “{query}”"))
            } else {
                None
            };
            for (action, label) in [
                (Action::SearchNext, "Next match"),
                (Action::SearchPrev, "Previous match"),
            ] {
                let mut row = here(action, label, "");
                row.unavailable.clone_from(&why);
                out.push(row);
            }
        }
        if let Route::Code(code::Commit { pr: Some(_), .. }) = route {
            out.push(here(Action::NextCommit, "Next commit", "next commit"));
            out.push(here(Action::PrevCommit, "Previous commit", ""));
        }
        if matches!(
            route,
            Route::Code(code::Blob { .. }) | Route::Code(code::Blame { .. })
        ) {
            let blame = match route {
                Route::Code(code::Blob { .. }) => "Blame this line",
                _ => "Back to the file",
            };
            out.push(here(Action::Blame, blame, "blame"));
            out.push(here(
                Action::VisualLines,
                "Select lines (l copies their link)",
                "",
            ));
        }
        if let Route::Search(search::Notifications { query }) = route {
            let show = format!("Show {}", next_list_state(route, query));
            out.push(here(Action::ToggleState, &show, "unread/read"));
        }
        // A word cursor moves along a line of code.
        if matches!(
            route,
            Route::Code(code::Blob { .. })
                | Route::Code(code::Blame { .. })
                | Route::Runs(runs::Job { .. })
        ) {
            let why = (p.word_cursor().is_none()).then(|| "no line of code is selected".to_owned());
            for action in [Action::WordNext, Action::WordPrev, Action::SearchWord] {
                let mut row = here(action, action.description(), "");
                row.unavailable.clone_from(&why);
                out.push(row);
            }
        }
        // Code navigation: a file's symbols, and a name's definition.
        if matches!(
            route,
            Route::Code(code::Blob { .. }) | Route::Code(code::Blame { .. })
        ) {
            out.push(here(Action::Symbols, "Symbols in this file", ""));
            let mut definition = here(Action::Definition, Action::Definition.description(), "");
            if p.word_cursor().is_none() {
                definition.unavailable = Some("no line of code is selected".into());
            }
            let definition_unavailable = definition.unavailable.clone();
            out.push(definition);
            out.push(here(Action::RepoSymbols, "Symbols in the repository", ""));
            let mut references = here(Action::References, Action::References.description(), "");
            references.unavailable.clone_from(&definition_unavailable);
            out.push(references);
        }
        if let Some(query) = &list {
            // Open, closed and sorting are issues', pull requests' and
            // discussions'; repositories, people, commits and code have none.
            if super::lists::plain_search(route).is_none() {
                let show = format!("Show {}", next_list_state(route, query));
                out.push(here(Action::ToggleState, &show, "open/closed"));
                out.push(here(Action::Sort, "Change the sort", "sort"));
            }
        }
        if let Some(repo) = route.repo() {
            out.push(here(Action::FindFile, "Go to file", "go to file"));
            if matches!(
                route,
                Route::Code(code::Repo(_))
                    | Route::Code(code::Tree { .. })
                    | Route::Code(code::Blob { .. })
            ) {
                out.push(here(Action::Branch, "Switch branches or tags", "branch"));
                let starred = self.overview(repo).is_some_and(|o| o.starred);
                let label = if starred { "Unstar" } else { "Star" };
                out.push(here(Action::Star, label, "star"));
            }
        }
        if matches!(
            route,
            Route::Issues(issues::Issue { .. })
                | Route::Pulls(pulls::Pr { .. })
                | Route::Code(code::Commit { .. })
        ) {
            out.push(here(Action::Comment, "Write a comment", "comment"));
            let mut quote = here(Action::QuoteReply, "Reply quoting the selected comment", "");
            if !matches!(p.selected_link(), Some(Link::Quote { .. })) {
                quote.unavailable = Some("no comment is selected".into());
            }
            out.push(quote);
        }
        // Long reviews fold, in a conversation and on a review's page.
        let folds = p
            .page
            .links
            .iter()
            .any(|l| matches!(l, Link::Review(crate::ui::page::ReviewLink::Fold(_))));
        if folds {
            out.push(here(
                Action::FoldToggle,
                "Fold or unfold the review",
                "fold",
            ));
        }
        if let Route::Pulls(pulls::Pr { tab, .. }) = route {
            if *tab == PrTab::Conversation {
                out.push(here(Action::Tab2, "Commits", ""));
            }
            out.push(here(Action::Tab4, "Files changed (review)", ""));
        }
    }

    /// Whether `action` does something here, or why not. What the menu
    /// lists decides, so which-key, the palette and the menu agree; moving
    /// and going places work everywhere.
    pub fn availability(&self, action: Action) -> Result<(), String> {
        use Action::*;
        if let Some(d) = self.doables().into_iter().find(|d| d.action == action) {
            return d.unavailable.map_or(Ok(()), Err);
        }
        let everywhere = action.moves()
            || Action::OPEN_TABS.contains(&action)
            || matches!(
                action,
                Back | Forward
                    | Quit
                    | Menu
                    | CommandPalette
                    | Messages
                    | Search
                    | SearchNext
                    | SearchPrev
                    | SearchGithub
                    | GoHome
                    | Notifications
                    | Refresh
                    | Copy
                    | OpenInBrowser
                    | OpenLink
                    | OpenInTab
                    | NextOpenTab
                    | PrevOpenTab
                    | CloseTab
                    | UpLevel
            );
        if everywhere {
            Ok(())
        } else {
            Err("not here".to_owned())
        }
    }

    /// The status bar's key hints.
    pub fn key_hints(&self) -> Vec<(String, String)> {
        let pair = |keys: &str, what: &str| (keys.to_owned(), what.to_owned());
        match &self.overlay {
            Some(Overlay::Search(sb)) => {
                let enter = match self.route() {
                    _ if !sb.filter => "go",
                    Some(Route::Runs(runs::Job { .. })) => "search",
                    _ => "filter",
                };
                return vec![pair("↵", enter), pair("↑↓", "move"), pair("esc", "close")];
            }
            Some(Overlay::Hints(_)) => {
                return vec![pair("a-z", "type a link's letters"), pair("esc", "cancel")];
            }
            Some(Overlay::Menu(menu)) if menu.filter.is_some() => {
                return vec![
                    pair("↵", "run"),
                    pair("↑↓", "choose"),
                    pair("esc", "stop filtering"),
                ];
            }
            Some(Overlay::Menu(_)) => {
                return vec![
                    pair("↵", "run"),
                    pair("↑↓", "choose"),
                    pair("key", "run it"),
                    (self.first_key(Action::Search), "filter".into()),
                    pair("esc", "close"),
                ];
            }
            Some(Overlay::Picker(p)) => {
                use crate::edit::Choosing;
                use crate::picker::Kind;
                // What ↵ does depends on what's being picked.
                let enter = match &p.kind {
                    Kind::Edit(Choosing::Title { .. }) | Kind::SectionTitle(..) => "save",
                    Kind::Edit(Choosing::Many { .. }) => "pick or unpick · ctrl-s apply",
                    Kind::Form(pick) if pick.several() => "pick or unpick",
                    Kind::Edit(_)
                    | Kind::Form(_)
                    | Kind::Project(_)
                    | Kind::Alert(_)
                    | Kind::Watch(_)
                    | Kind::Tag(_)
                    | Kind::Target(_)
                    | Kind::SaveAsset(_)
                    | Kind::Upload(_)
                    | Kind::Description(_)
                    | Kind::FileName(..)
                    | Kind::Site(_)
                    | Kind::Housekeeping(_)
                    | Kind::Workflows(_)
                    | Kind::Manage(_)
                    | Kind::Settings(_) => "choose",
                    Kind::Code { .. } => "go there",
                    Kind::Messages(_) => "copy",
                    Kind::Commands
                    | Kind::Files { .. }
                    | Kind::Branches { .. }
                    | Kind::DiffFiles
                    | Kind::Commits { .. } => "open",
                };
                return vec![pair("↵", enter), pair("↑↓", "move"), pair("esc", "close")];
            }
            Some(_) => return Vec::new(),
            None => {}
        }
        // The menu first: everything else is in it, so it must never be
        // the hint a narrow terminal drops. Then what fits the thing under
        // the cursor, then the screen's usual verbs.
        let menu = (self.first_key(Action::Menu), "menu".to_owned());
        let focused = self.focus_hints();
        // The file tree's keys are all its own.
        let tree = self.contexts().first() == Some(&crate::keymap::Context::Tree);
        let usual = self
            .doables()
            .into_iter()
            .filter(|_| !tree)
            .filter(|d| d.unavailable.is_none() && !d.short.is_empty())
            .filter(|d| !focused.iter().any(|(a, _)| *a == d.action))
            .map(|d| (d.action, d.short.to_owned()));
        let hints = focused
            .iter()
            .cloned()
            .chain(usual)
            .filter_map(|(action, what)| {
                // Pairs read as one hint.
                let pair = |a, b| Some(format!("{}/{}", self.key_here(a)?, self.key_here(b)?));
                let key = match action {
                    Action::NextTab => pair(Action::NextTab, Action::PrevTab),
                    Action::NextHunk => pair(Action::NextHunk, Action::PrevHunk),
                    Action::NextFile => pair(Action::NextFile, Action::PrevFile),
                    Action::NextSection => pair(Action::NextSection, Action::PrevSection),
                    Action::NextColumn => pair(Action::PrevColumn, Action::NextColumn),
                    Action::SearchNext => pair(Action::SearchNext, Action::SearchPrev),
                    action => self.key_here(action),
                };
                Some((key?, what))
            })
            .take(7);
        std::iter::once(menu).chain(hints).collect()
    }

    /// Verbs for what's under the diff's cursor (a thread, a draft) and for
    /// a review in progress, most useful first.
    pub(super) fn focus_hints(&self) -> Vec<(Action, String)> {
        let Screen::Diff(screen) = self.screen() else {
            return Vec::new();
        };
        let Some(diff) = self.diffs.get(&screen.of) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // The file tree has its own keys.
        if screen.focus == Pane::Tree && screen.prefs.tree_visible {
            return vec![
                (Action::Open, "show file".to_owned()),
                (Action::Fold, "fold".to_owned()),
                (Action::Unfold, "unfold".to_owned()),
                (Action::SwitchPane, "to the diff".to_owned()),
            ];
        }
        let here = diff
            .doc
            .annotation_under(screen.cursor, screen.half)
            .and_then(|i| diff.doc.annotations().get(i as usize));
        match here {
            Some(a) if a.is_draft() => {
                out.push((Action::Open, "edit".to_owned()));
                out.push((Action::Delete, "delete".to_owned()));
            }
            Some(a) => {
                if a.can_reply {
                    out.push((Action::Comment, "reply".to_owned()));
                }
                if a.resolved && a.can_unresolve {
                    out.push((Action::ResolveThread, "unresolve".to_owned()));
                } else if !a.resolved && a.can_resolve {
                    out.push((Action::ResolveThread, "resolve".to_owned()));
                }
            }
            None => {}
        }
        // A removed line beside an added one: the other is a key away.
        if let Some(Row::Split {
            left: Some(left),
            right: Some(right),
        }) = diff.doc.row(screen.cursor)
            && left != right
        {
            match diff.doc.half_at(screen.cursor, screen.half) {
                Some(Side::Left) => out.push((Action::NewSide, "new side".to_owned())),
                Some(Side::Right) => out.push((Action::OldSide, "old side".to_owned())),
                None => {}
            }
        }
        let pending = diff.inputs().review.pending.len();
        if pending > 0 {
            out.push((Action::SubmitReview, format!("submit ({pending} pending)")));
        }
        out
    }

    /// The first key that runs `action` on this screen, if any.
    pub fn key_here(&self, action: Action) -> Option<String> {
        let key = self.keymap.keys_in(action, self.contexts()).next()?;
        Some(crate::keymap::pretty(key))
    }
}

/// What following `link` does, for the menu.
pub(super) fn describe(link: &Link) -> String {
    match link {
        Link::Url(url) => match Dest::from_url(url).target {
            Target::Page(r) => format!("Open {}", r.title()),
            Target::Files(of) => format!("Files changed in {of}"),
            Target::External(u) => format!("Open {u} in the browser"),
        },
        Link::More => "Load more".into(),
        Link::Comment => "Write a comment".into(),
        Link::Star => "Star or unstar".into(),
        Link::Filter => "Edit the filter".into(),
        Link::Sort => "Change the sort".into(),
        Link::State(state) => format!("Show {state}"),
        Link::Branch => "Switch branches".into(),
        Link::FindFile => "Go to file".into(),
        Link::Quote { .. } => "Quote reply".into(),
        Link::Section(_) => "Open the section's list".into(),
        Link::Form(row) => row.describe().into(),
        Link::Artifact { .. } => "Save the artifact".into(),
        Link::Asset(_) => "Save the asset".into(),
        Link::Review(link) => crate::talk::describe(link).into(),
        Link::Setting(link) => crate::settings::describe(link).into(),
        Link::Queued(_) => "Open the page it was written on".into(),
    }
}
