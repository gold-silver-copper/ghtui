//! The page on screen: its selection and scrolling.

use super::*;
use crate::route::releases;

/// What a page was built for: the data's generation, its diff's inputs
/// (a pull request's Review tab shows them), the width and the density.
pub(super) type Built = (u64, Option<u64>, u16, bool);

/// The screen of one page: where it is, what's selected, and the page as
/// last built.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageScreen {
    pub route: Route,
    /// The top row shown.
    pub scroll: usize,
    /// The selected item, if one is.
    pub selected: Option<usize>,
    pub(super) page: Arc<Page>,
    pub(super) built: Option<Built>,
    /// Nothing has been selected or scrolled yet: select the first visible
    /// item once the page has items.
    pub(super) fresh: bool,
    /// Where its [`Page::jump`] or anchor last left the scroll and the
    /// selection: followed while both stay put, `Some(None)` once moved.
    pub(super) jumped: Option<Option<(usize, Option<usize>)>>,
    /// The `#fragment` of the link that opened it: a comment to scroll to.
    pub anchor: Option<String>,
    /// The search match last gone to, by its index in [`Page::marks`].
    pub(super) mark: Option<usize>,
    /// What `/` searched the page for (a job's log keeps its own).
    pub find: Option<String>,
    /// Where a selection of a file's lines started (`V`), by item.
    pub range: Option<usize>,
    /// Columns code is scrolled left by.
    pub shift: usize,
    /// How far code can scroll left ([`Page::wide`]), found once a build.
    pub(super) wide: usize,
    /// The word cursor, by its column in the code of the selected line:
    /// `None` until `w` or `b` picks one.
    pub word: Option<usize>,
    /// A name the word cursor goes to once its line is selected and
    /// built (where `gd` lands).
    pub land: Option<String>,
}

impl PageScreen {
    pub fn new(route: Route, anchor: Option<String>) -> Self {
        // Lists start on their first row, a job on a step and a file on the
        // lines linked; reading pages (issues, pull requests, files) start
        // with nothing selected.
        let list = route.starts_on_row()
            && !matches!(
                route,
                Route::Releases(releases::Release { .. }) | Route::Discussion { .. }
            );
        Self {
            route,
            scroll: 0,
            selected: None,
            page: Arc::new(Page::default()),
            built: None,
            fresh: list,
            jumped: None,
            anchor,
            mark: None,
            find: None,
            range: None,
            shift: 0,
            wide: 0,
            word: None,
            land: None,
        }
    }

    /// The page as last built.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// Shows a new build of the page, keeping the selection and the view
    /// on the same items.
    pub(super) fn show(&mut self, mut page: Page, built: Built) {
        if let Some(query) = &self.find {
            page.mark(query);
        }
        self.wide = page.wide();
        let old = mem::replace(&mut self.page, Arc::new(page));
        self.built = Some(built);
        let before = (self.scroll, self.selected);
        let find = |i: usize| self.page.find(old.target(old.items.get(i)?.link)?, i);
        // A fresh page selects its first visible item again. An item that
        // left (a pull request closed off a list) leaves the selection
        // where it was, on the one after it.
        let last = self.page.items.len().checked_sub(1);
        self.selected = (self.selected.filter(|_| !self.fresh))
            .and_then(|i| find(i).or_else(|| last.map(|l| i.min(l))));
        // The item still there starting nearest the top, if nearer than
        // the page's top, keeps its place on screen.
        let off = |it: &Item| it.start.abs_diff(self.scroll);
        let mut near: Vec<_> = old.items.iter().enumerate().collect();
        near.sort_by_key(|&(_, it)| off(it));
        let mut kept = near.into_iter().filter(|&(_, it)| off(it) < self.scroll);
        let moved = kept.find_map(|(i, it)| Some((it, self.page.items.get(find(i)?)?)));
        if let Some((it, now)) = moved {
            self.scroll = (self.scroll + now.start).saturating_sub(it.start);
        }
        // Following the same content isn't moving away from a jump.
        if self.jumped == Some(Some(before)) {
            self.jumped = Some(Some((self.scroll, self.selected)));
        }
    }

    /// The search match gone to, by its index in [`Page::marks`]: before
    /// any, the one the page opened at.
    pub(super) fn mark_at(&self) -> Option<usize> {
        let marks = &self.page.marks;
        (self.mark.filter(|&m| m < marks.len()))
            .or_else(|| marks.iter().position(|&m| Some(m) == self.page.jump))
    }

    /// The line of the search match gone to.
    pub fn current_match(&self) -> Option<usize> {
        self.page.marks.get(self.mark_at()?).copied()
    }

    /// Selects item `i` and scrolls it into view, as moving to it does.
    pub fn select(&mut self, i: usize, height: usize) {
        self.fresh = false;
        self.selected = Some(i);
        reveal(self, i, height);
    }

    /// The file line item `i` starts at, from 1.
    pub fn line_of(&self, i: usize) -> Option<u32> {
        let n = self.page.numbered.iter().position(|&it| it == i)?;
        u32::try_from(n + 1).ok()
    }

    /// The selected item's link.
    pub fn selected_link(&self) -> Option<&Link> {
        let item = self.page.items.get(self.selected?)?;
        self.page.target(item.link)
    }

    /// The selected row's code (a line of a file or a log): its line, its
    /// code, and the columns the code has on screen.
    pub(super) fn code_line(&self) -> Option<(usize, String, usize)> {
        let at = self.page.items.get(self.selected?)?.start;
        let line = self.page.lines.get(at)?;
        let (code, before) = line.code()?;
        let room = usize::from(self.page.room(line.frame, line.indent)).saturating_sub(before);
        Some((at, code, room))
    }

    /// The word under the cursor (before `w` or `b` picks one, the first
    /// of the line), with its line and the line's code.
    pub fn word_cursor(&self) -> Option<(usize, String, Word)> {
        let (at, code, _) = self.code_line()?;
        let word = text::word_at(&text::words(&code), self.word.unwrap_or(0))?.clone();
        Some((at, code, word))
    }

    /// The word under the cursor.
    pub(super) fn word_text(&self) -> Option<String> {
        let (_, code, word) = self.word_cursor()?;
        code.get(word.bytes).map(str::to_owned)
    }
}

impl State {
    /// Where pages draw: the content area below a row of breathing room.
    pub fn page_area(&self) -> Rect {
        let c = self.layout().content;
        Rect {
            y: c.y + PAD_Y.min(c.height),
            height: c.height.saturating_sub(PAD_Y),
            ..c
        }
    }

    /// Columns pages are built for: each takes what it fills of them.
    pub fn page_width(&self) -> u16 {
        self.size.0.saturating_sub(2 * PAD_X + 1)
    }

    /// Rows a page shows.
    pub fn page_height(&self) -> usize {
        usize::from(self.page_area().height).max(1)
    }

    /// Rebuilds the page on screen if its data or the width changed, and
    /// keeps its scroll and selection valid.
    pub fn sync_page(&mut self) {
        let width = self.page_width();
        let height = self.page_height();
        let Screen::Page(p) = self.screen() else {
            return;
        };
        let built = (
            self.data_gen,
            self.diff_version(&p.route),
            width,
            self.compact(),
        );
        let rebuild = (p.built != Some(built)).then(|| p.route.clone());
        let page = rebuild.map(|route| self.build_page(&route, width, (self.clock)()));
        if let Screen::Page(p) = self.screen_mut() {
            if let Some(page) = page {
                p.show(page, built);
            }
            settle(p, height);
        }
    }
}

/// Whether item `i` is a row to act on, not a note (an empty Home
/// section's, which opens its list).
pub(super) fn is_row(p: &PageScreen, i: usize) -> bool {
    let link = p.page.items.get(i).and_then(|it| p.page.target(it.link));
    !matches!(link, Some(Link::Section(_)))
}

pub(super) fn visible(p: &PageScreen, item: usize, height: usize) -> bool {
    p.page
        .items
        .get(item)
        .is_some_and(|i| i.start < p.scroll + height && i.end > p.scroll)
}

/// Keeps a page's scroll in range and follows its jump or anchor; a fresh
/// page selects its first visible item.
pub(super) fn settle(p: &mut PageScreen, height: usize) {
    let anchor = p
        .anchor
        .as_ref()
        .and_then(|a| p.page.anchors.get(a))
        .copied();
    let max = p.page.height().saturating_sub(height);
    // Again when a rebuild moves it, until you scroll or select.
    let here = Some((p.scroll, p.selected));
    let jump = (p.page.jump.or(anchor)).filter(|_| p.jumped.is_none_or(|at| at == here));
    if let Some(jump) = jump {
        p.scroll = jump.saturating_sub(MARGIN);
    }
    p.scroll = p.scroll.min(max);
    if p.fresh && p.selected.is_none() {
        // A row of a list, before a Home section's note (its empty state).
        let shown = |i: &usize| visible(p, *i, height);
        let row = |i: &usize| is_row(p, *i);
        let n = p.page.items.len();
        p.selected = p.page.selects.filter(|&i| i < n).or_else(|| {
            (0..n)
                .filter(shown)
                .find(row)
                .or_else(|| (0..n).find(shown))
        });
    }
    if jump.is_some() || p.jumped.is_some() {
        p.jumped = Some(jump.map(|_| (p.scroll, p.selected)));
    }
    // The name `gd` went to, once its line is there.
    if let Some(name) = p.land.clone()
        && let Some((_, code, _)) = p.code_line()
    {
        let words = text::words(&code);
        let at = words
            .iter()
            .find(|w| code.get(w.bytes.clone()) == Some(name.as_str()));
        p.word = at.map(|w| w.cols.start).or(p.word);
        p.land = None;
    }
}

pub(super) fn reveal(p: &mut PageScreen, item: usize, height: usize) {
    if let Some(i) = p.page.items.get(item) {
        p.scroll = page::reveal(i.start, i.end, p.scroll, height, p.page.height(), MARGIN);
    }
}

/// `↓`/`↑`: more of the selected item if it continues off screen; else the
/// next item; else (past the last one) scroll on.
pub(super) fn step(p: &mut PageScreen, height: usize, down: bool) {
    p.fresh = false;
    let n = p.page.items.len();
    let items = &p.page.items;
    let target = match p.selected.map(|i| (i, items.get(i))) {
        Some((_, Some(item))) if down && item.end > p.scroll + height => None,
        Some((_, Some(item))) if !down && item.start < p.scroll => None,
        Some((i, _)) if down => (i + 1 < n).then_some(i + 1),
        Some((i, _)) => i.checked_sub(1),
        // Reading: pick up the first item that's on screen.
        None if down => (0..n).find(|&i| visible(p, i, height)),
        None => (0..n).rev().find(|&i| visible(p, i, height)),
    };
    match target {
        Some(i) => {
            p.selected = Some(i);
            reveal(p, i, height);
        }
        None if down => scroll_by(p, STEP, height),
        None => scroll_by(p, -STEP, height),
    }
}

/// Scrolls; a selection that leaves the screen is dropped.
pub(super) fn scroll_by(p: &mut PageScreen, rows: isize, height: usize) {
    p.fresh = false;
    let max = p.page.height().saturating_sub(height);
    p.scroll = p.scroll.saturating_add_signed(rows).min(max);
    if p.selected.is_some_and(|s| !visible(p, s, height)) {
        p.selected = None;
    }
}

/// Pages and jumps keep a selection on screen when there's one to keep:
/// one scrolled off is the first item shown on that side. When the view
/// can't scroll that way, the selection goes to that edge.
pub(super) fn scroll_keep(p: &mut PageScreen, rows: isize, height: usize) {
    let had = p.selected.is_some();
    let scroll = p.scroll;
    scroll_by(p, rows, height);
    let stuck = p.scroll == scroll;
    if (had && p.selected.is_none()) || stuck {
        let mut visible_items = (0..p.page.items.len()).filter(|&i| visible(p, i, height));
        p.selected = if (rows > 0) == stuck {
            visible_items.next_back()
        } else {
            visible_items.next()
        };
    }
}
