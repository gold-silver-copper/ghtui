//! What you can do here: the actions menu's rows and the keys hinted in
//! the status bar.

use super::*;

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

pub(super) fn doable(
    section: &'static str,
    action: Action,
    label: &str,
    short: &'static str,
) -> Doable {
    Doable {
        section,
        action,
        label: label.to_owned(),
        short,
        unavailable: None,
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
                (A::SearchBack, ""),
                (A::SearchNext, ""),
                (A::SearchPrev, ""),
                (A::SearchWord, ""),
                (A::SearchWordBack, ""),
                (A::WordNext, ""),
                (A::WordPrev, ""),
                (A::FindFile, ""),
                (A::SwitchPane, ""),
                (A::Fold, ""),
                (A::Unfold, ""),
            ],
        ),
        (
            "View",
            &[
                (A::FoldClose, ""),
                (A::FoldOpen, ""),
                (A::FoldToggle, ""),
                (A::FoldAll, ""),
                (A::UnfoldAll, ""),
                (A::ToggleTree, ""),
                (A::TreeNarrower, ""),
                (A::TreeWider, ""),
                (A::ToggleSplit, ""),
                (A::IgnoreWhitespace, ""),
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
                        out.push(doable(section, action, action.description(), short));
                    }
                }
                None
            }
        };
        for (action, label, why) in crate::act::doables(self) {
            let short = match action {
                Action::Merge => "merge",
                Action::Approve => "approve",
                Action::Rerun => "re-run",
                Action::MarkRead => "read",
                Action::MarkDone => "done",
                _ => "",
            };
            let mut change = doable("Change", action, &label, short);
            change.unavailable = why;
            out.push(change);
        }
        let go = |action, label: &str, short| doable("Go", action, label, short);
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
        let close = if self.screens.len() > 1 {
            "Close this page (back)"
        } else {
            "Close the tab"
        };
        let mut close = go(Action::CloseView, close, "");
        if self.screens.len() < 2 && self.tab_count() < 2 {
            close.unavailable = Some("nothing left to close".into());
        }
        out.push(close);
        out.push(go(Action::GoHome, "Home", ""));
        out.push(go(Action::Notifications, "Notifications", ""));
        out.push(go(Action::Refresh, "Refresh", ""));
        out.push(go(Action::CommandPalette, "Command palette", ""));
        let mut messages = go(Action::Messages, "Recent messages and errors", "");
        if self.notices.log().is_empty() {
            messages.unavailable = Some("nothing yet".into());
        }
        out.push(messages);
        let tab = |action, label: &str| doable("Tabs", action, label, "");
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
            Action::NextItem,
            Action::PrevItem,
            Action::NextSection,
            Action::PrevSection,
            Action::ScreenTop,
            Action::ScreenMiddle,
            Action::ScreenBottom,
            Action::Top,
            Action::Bottom,
            Action::HalfPageDown,
            Action::HalfPageUp,
            Action::PageDown,
            Action::PageUp,
            Action::ScrollCenter,
            Action::ScrollLeft,
            Action::ScrollRight,
            Action::Cancel,
            Action::Quit,
        ] {
            if contexts.contains(&action.context()) || self.key_here(action).is_some() {
                out.push(doable("Keys", action, action.description(), ""));
            }
        }
        out
    }

    pub(super) fn page_doables(&self, p: &PageScreen, out: &mut Vec<Doable>) {
        let here = |action, label: &str, short| doable("This page", action, label, short);
        let route = &p.route;
        // A file's line is this page, as a log's is: nothing to open.
        let line = matches!(route, Route::Job { .. }) && p.code_line().is_some();
        if !matches!(route, Route::Blob { .. }) && !line {
            let mut open = here(Action::Open, "Open the selected row", "open");
            match p.selected_link() {
                Some(link) => open.label = describe(link),
                None => open.unavailable = Some("nothing is selected".into()),
            }
            out.push(open);
        }
        let list = route.search().map(|(_, q)| q);
        // `/` searches the page, or filters a list; then n/N.
        let searched = match route {
            Route::Job { query, .. } => {
                out.push(here(Action::Search, "Search the log", "search"));
                Some(query.clone())
            }
            _ if route.list_query().is_some() => {
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
        if let Route::Commit { pr: Some(_), .. } = route {
            out.push(here(Action::NextCommit, "Next commit", "next commit"));
            out.push(here(Action::PrevCommit, "Previous commit", ""));
        }
        if matches!(route, Route::Blob { .. } | Route::Blame { .. }) {
            let blame = match route {
                Route::Blob { .. } => "Blame this line",
                _ => "Back to the file",
            };
            out.push(here(Action::Blame, blame, "blame"));
            out.push(here(
                Action::VisualLines,
                "Select lines (y copies their link)",
                "",
            ));
        }
        if let Route::Notifications { query } = route {
            let show = format!("Show {}", next_list_state(route, query));
            out.push(here(Action::ToggleState, &show, "unread/read"));
        }
        // A word cursor moves along a line of code.
        if matches!(
            route,
            Route::Blob { .. } | Route::Blame { .. } | Route::Job { .. }
        ) {
            let why = (p.word_cursor().is_none()).then(|| "no line of code is selected".to_owned());
            for action in [
                Action::WordNext,
                Action::WordPrev,
                Action::SearchWord,
                Action::SearchWordBack,
            ] {
                let mut row = here(action, action.description(), "");
                row.unavailable.clone_from(&why);
                out.push(row);
            }
        }
        if let Some(query) = &list {
            let show = format!("Show {}", next_list_state(route, query));
            out.push(here(Action::ToggleState, &show, "open/closed"));
            out.push(here(Action::Sort, "Change the sort", "sort"));
            let mut save = here(Action::SaveSection, "Save this list to Home", "");
            save.unavailable = crate::home::unavailable(self, Action::SaveSection);
            out.push(save);
        }
        if *route == Route::Home {
            let title = match self.section_here() {
                Some((_, s)) => format!("“{}”", s.title),
                None => "the section".to_owned(),
            };
            for (action, label) in [
                (Action::RenameSection, format!("Rename {title}")),
                (Action::MoveSectionUp, format!("Move {title} up")),
                (Action::MoveSectionDown, format!("Move {title} down")),
                (Action::Delete, format!("Remove {title} from Home")),
                (
                    Action::UndoDelete,
                    "Bring back the section removed".to_owned(),
                ),
            ] {
                let mut row = doable("Home", action, &label, "");
                row.unavailable = crate::home::unavailable(self, action);
                out.push(row);
            }
        }
        if let Some(repo) = route.repo() {
            out.push(here(Action::FindFile, "Go to file", "go to file"));
            if matches!(
                route,
                Route::Repo(_) | Route::Tree { .. } | Route::Blob { .. }
            ) {
                out.push(here(Action::Branch, "Switch branches or tags", "branch"));
                let starred = self.overview(repo).is_some_and(|o| o.starred);
                let label = if starred { "Unstar" } else { "Star" };
                out.push(here(Action::Star, label, "star"));
            }
        }
        if matches!(route, Route::Issue { .. } | Route::Pr { .. }) {
            out.push(here(Action::Comment, "Write a comment", "comment"));
            let mut quote = here(Action::QuoteReply, "Reply quoting the selected comment", "");
            if !matches!(p.selected_link(), Some(Link::Quote { .. })) {
                quote.unavailable = Some("no comment is selected".into());
            }
            out.push(quote);
        }
        if let Route::Pr { tab, .. } = route {
            if *tab == PrTab::Conversation {
                out.push(here(Action::Tab2, "Commits", ""));
            }
            out.push(here(Action::Tab4, "Files changed (review)", ""));
        }
    }

    /// After a count or the first keys of a sequence: what's typed, and
    /// what can follow it here.
    pub fn which_key(&self) -> Option<(String, Vec<KeyRow>)> {
        let count = self.count.map(|c| c.to_string()).unwrap_or_default();
        if self.typed.is_empty() {
            self.count?;
            let tab = self.first_key(Action::NextTab);
            let top = self.first_key(Action::Top);
            let rows = vec![
                KeyRow::key("j k …", "that many times"),
                KeyRow::key(tab, format!("tab {count}")),
                KeyRow::key(top, format!("line {count}")),
            ];
            return Some((format!("{count}…"), rows));
        }
        let typed = format!("{count}{}…", crate::keymap::pretty_keys(&self.typed));
        let rows = (self
            .keymap
            .continuations(&self.typed, self.contexts())
            .into_iter())
        .filter(|(_, a)| self.availability(*a).is_ok())
        .map(|(rest, a)| KeyRow::key(crate::keymap::pretty_keys(&rest), a.description()))
        .collect();
        Some((typed, rows))
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
                    | CloseView
                    | Quit
                    | Menu
                    | CommandPalette
                    | Messages
                    | Search
                    | SearchBack
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
                    Some(Route::Job { .. }) => "search",
                    _ => "filter",
                };
                return vec![
                    pair("↵", enter),
                    pair("ctrl-j/k", "choose"),
                    pair("esc", "close"),
                ];
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
                    pair("j/k", "choose"),
                    pair("key", "run it"),
                    (self.first_key(Action::Search), "filter".into()),
                    pair("esc", "close"),
                ];
            }
            Some(Overlay::Picker(_)) => {
                return vec![
                    pair("↵", "open"),
                    pair("ctrl-j/k", "choose"),
                    pair("esc", "close"),
                ];
            }
            Some(_) => return Vec::new(),
            None => {}
        }
        // Keys typed so far: the which-key panel lists what can follow.
        if let Some((typed, _)) = self.which_key() {
            return vec![(typed, String::new()), pair("esc", "cancel")];
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
            Some(_) => {
                out.push((Action::Comment, "reply".to_owned()));
                out.push((Action::ResolveThread, "resolve".to_owned()));
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

    /// The first keys that run `action` on this screen, if any. The
    /// page's tabs by number are a count before the next tab's keys.
    pub fn key_here(&self, action: Action) -> Option<String> {
        let stack = self.contexts();
        if let Some(keys) = self.keymap.keys_in(action, stack).next() {
            return Some(crate::keymap::pretty_keys(keys));
        }
        let n = Action::PAGE_TABS.iter().position(|a| *a == action)? + 1;
        let next = self.keymap.keys_in(Action::NextTab, stack).next()?;
        Some(format!("{n}{}", crate::keymap::pretty_keys(next)))
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
    }
}
