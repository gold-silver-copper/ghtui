//! What you can do here: the actions menu's rows and the keys hinted in
//! the status bar.

use super::*;
use crate::route::{code, issues, pulls, runs, search};
use std::borrow::Cow;

/// Something you can do here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Doable {
    pub section: &'static str,
    pub action: Action,
    /// For the menu: most are written in, so the status bar's rebuild
    /// each frame copies none.
    pub label: Cow<'static, str>,
    /// For the status bar; empty keeps it out.
    pub short: &'static str,
    pub unavailable: Option<String>,
}

/// A menu row.
pub(crate) fn row(
    section: &'static str,
    action: Action,
    label: impl Into<Cow<'static, str>>,
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

impl Doable {
    /// Unavailable, for `why`, when `not`.
    fn unavailable_if(mut self, not: bool, why: &str) -> Self {
        if not {
            self.unavailable = Some(why.to_owned());
        }
        self
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
                (A::WrapLines, ""),
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
        let go = |action, label: &'static str, short| row("Go", action, label, short, None);
        out.push(go(
            Action::SearchGithub,
            "Search GitHub or go to a page",
            "",
        ));
        // Only the screen's own tabs count here, not the header's right.
        let chrome = self.screen_chrome();
        if !chrome.tabs.is_empty() {
            out.push(go(Action::NextTab, "Next tab", "tabs"));
            out.push(go(Action::PrevTab, "Previous tab", ""));
        }
        if let Screen::Diff(_) = self.screen() {
            out.push(go(Action::Tab1, "Back to the conversation", ""));
        }
        if let Some(n) = review_tab(&chrome) {
            let glance = "The review at a glance: drafts, threads, reviewers";
            let glance = go(Action::ReviewGlance, glance, "");
            out.push(glance.unavailable_if(chrome.active == Some(n - 1), "this is it"));
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
        let link = go(Action::OpenLink, Action::OpenLink.description(), "");
        let no_link = self.link_under_cursor().is_none();
        out.push(link.unavailable_if(no_link, "no link is under the cursor"));
        out.push(go(Action::Hints, "Follow any link by its letters", "links"));
        if let Screen::Page(_) = self.screen() {
            out.push(go(Action::UpLevel, "Up a level", ""));
        }
        let back = go(Action::Back, "Back", "");
        out.push(back.unavailable_if(self.screens.len() < 2, "this is the first page"));
        let fwd = go(Action::Forward, "Forward", "");
        out.push(fwd.unavailable_if(self.forward.is_empty(), "nothing to go forward to"));
        for (action, label) in [
            (Action::GoHome, "Home"),
            (Action::Notifications, "Notifications"),
            (Action::Explore, "Explore: new repositories and topics"),
            (Action::Refresh, "Refresh"),
            (Action::CommandPalette, "Command palette"),
        ] {
            out.push(go(action, label, ""));
        }
        let messages = go(Action::Messages, "Recent messages and errors", "");
        out.push(messages.unavailable_if(self.notices.log().is_empty(), "nothing yet"));
        let tab = |action, label: &'static str| row("Tabs", action, label, "", None);
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
        let here =
            |action, label: &'static str, short| row("This page", action, label, short, None);
        let route = &p.route;
        // The list state shown next: open or closed, unread or read.
        let toggle = |query: &str, short| {
            let show = format!("Show {}", next_list_state(route, query));
            row("This page", Action::ToggleState, show, short, None)
        };
        let file = matches!(route, Route::Code(code::Blob { .. } | code::Blame { .. }));
        let job = matches!(route, Route::Runs(runs::Job { .. }));
        // A file's line is this page, as a log's is: nothing to open.
        let line = job && p.code_line().is_some();
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
        if file {
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
            out.push(toggle(query, "unread/read"));
        }
        // A word cursor moves along a line of code; in a file, code
        // navigation goes from a name to its definition and references.
        if file || job {
            let no_word = p.word_cursor().is_none();
            let word = |action: Action| {
                let row = here(action, action.description(), "");
                row.unavailable_if(no_word, "no line of code is selected")
            };
            out.extend([Action::WordNext, Action::WordPrev, Action::SearchWord].map(word));
            if file {
                out.push(here(Action::Symbols, "Symbols in this file", ""));
                out.push(word(Action::Definition));
                out.push(here(Action::RepoSymbols, "Symbols in the repository", ""));
                out.push(word(Action::References));
            }
        }
        if let Some(query) = &list {
            // Open, closed and sorting are issues', pull requests' and
            // discussions'; repositories, people, commits and code have none.
            if super::lists::plain_search(route).is_none() {
                out.push(toggle(query, "open/closed"));
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
            let quote = here(Action::QuoteReply, "Reply quoting the selected comment", "");
            let unquotable = !matches!(p.selected_link(), Some(Link::Quote { .. }));
            out.push(quote.unavailable_if(unquotable, "no comment is selected"));
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
        availability_in(&self.doables(), action)
    }

    /// The status bar's key hints: what they do is mostly written in, so
    /// the rebuild each frame copies none.
    pub fn key_hints(&self) -> Vec<(String, Cow<'static, str>)> {
        let pair = |keys: &str, what: &'static str| (keys.to_owned(), what.into());
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
        let menu = (self.first_key(Action::Menu), "menu".into());
        let focused = self.focus_hints();
        // The file tree's keys are all its own.
        let tree = self.contexts().first() == Some(&crate::keymap::Context::Tree);
        let usual = ((!tree).then(|| self.doables()).into_iter().flatten())
            .filter(|d| d.unavailable.is_none() && !d.short.is_empty())
            .filter(|d| !focused.iter().any(|(a, _)| *a == d.action))
            .map(|d| (d.action, d.short.into()));
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
    pub(super) fn focus_hints(&self) -> Vec<(Action, Cow<'static, str>)> {
        let Screen::Diff(screen) = self.screen() else { return Vec::new() };
        let Some(diff) = self.diffs.get(&screen.of) else { return Vec::new() };
        let mut out = Vec::new();
        // The file tree has its own keys.
        if screen.focus == Pane::Tree && screen.prefs.tree_visible {
            return vec![
                (Action::Open, "show file".into()),
                (Action::Fold, "fold".into()),
                (Action::Unfold, "unfold".into()),
                (Action::SwitchPane, "to the diff".into()),
            ];
        }
        let here = diff
            .doc
            .annotation_under(screen.cursor, screen.half)
            .and_then(|i| diff.doc.annotations().get(i as usize));
        match here {
            Some(a) if a.is_draft() => {
                out.push((Action::Open, "edit".into()));
                out.push((Action::Delete, "delete".into()));
            }
            Some(a) => {
                if a.can_reply {
                    out.push((Action::Comment, "reply".into()));
                }
                if a.resolved && a.can_unresolve {
                    out.push((Action::ResolveThread, "unresolve".into()));
                } else if !a.resolved && a.can_resolve {
                    out.push((Action::ResolveThread, "resolve".into()));
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
                Some(Side::Left) => out.push((Action::NewSide, "new side".into())),
                Some(Side::Right) => out.push((Action::OldSide, "old side".into())),
                None => {}
            }
        }
        let pending = diff.inputs().review.pending.len();
        if pending > 0 {
            out.push((
                Action::SubmitReview,
                format!("submit ({pending} pending)").into(),
            ));
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
pub(super) fn describe(link: &Link) -> Cow<'static, str> {
    match link {
        Link::Url(url) => match Dest::from_url(url).target {
            Target::Page(r) => format!("Open {}", r.title()).into(),
            Target::Files(of) => format!("Files changed in {of}").into(),
            Target::External(u) => format!("Open {u} in the browser").into(),
        },
        Link::More => "Load more".into(),
        Link::Comment => "Write a comment".into(),
        Link::Star => "Star or unstar".into(),
        Link::Filter => "Edit the filter".into(),
        Link::Sort => "Change the sort".into(),
        Link::State(state) => format!("Show {state}").into(),
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

/// [`State::availability`], with the screen's [`State::doables`] already
/// built: the palette asks for every action at once.
pub fn availability_in(doables: &[Doable], action: Action) -> Result<(), String> {
    use Action::*;
    if let Some(d) = doables.iter().find(|d| d.action == action) {
        return d.unavailable.clone().map_or(Ok(()), Err);
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
