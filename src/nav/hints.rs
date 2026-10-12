//! Letter hints, for following links by letters.

use super::*;

pub struct Hints {
    pub labels: Vec<HintLabel>,
    pub links: Vec<Link>,
    /// The page tab each link is, if it's one (1-based).
    pub tabs: Vec<Option<usize>>,
    pub typed: String,
    pub browser: bool,
    /// A capital was typed: follow the link in a new tab.
    pub tab: bool,
}

pub(super) const LETTERS: &str = "asdfghjklqwertyuiopzxcvbnm";

/// Labels for `n` targets: single letters while they last, then pairs.
pub(super) fn hint_labels(n: usize) -> Vec<String> {
    let letters: Vec<char> = LETTERS.chars().collect();
    if n <= letters.len() {
        return letters
            .iter()
            .take(n)
            .map(std::string::ToString::to_string)
            .collect();
    }
    // Spread over as many first letters as possible, so one key narrows the
    // choice to a few.
    let per = n.div_ceil(letters.len()).min(letters.len());
    letters
        .iter()
        .flat_map(|a| letters.iter().take(per).map(move |b| format!("{a}{b}")))
        .take(n)
        .collect()
}

pub(super) fn start_hints(state: &mut State, browser: bool) {
    let area = state.page_area();
    let Screen::Page(p) = state.screen() else {
        return;
    };
    // The page's own tabs first, then its links.
    let chrome = state.chrome();
    let tab_list: Vec<_> = chrome.tabs.iter().map(|(t, _)| t.clone()).collect();
    let tab_rects = (state.layout_of(&chrome).tabs)
        .map(|bar| chrome::tab_layout(bar, &tab_list, chrome.active))
        .unwrap_or_default();
    let tabs = (chrome.tabs.iter().zip(tab_rects).enumerate())
        .filter(|(i, _)| chrome.active != Some(*i))
        .map(|(i, ((_, target), rect))| {
            let url = match target {
                Target::Page(route) => route.url(),
                Target::Files(of) => of.url(),
                Target::External(url) => url.clone(),
            };
            (
                (rect.x.saturating_add(2), rect.y),
                Link::Url(url),
                Some(i + 1),
            )
        });
    let links = page::spots(&p.page, area, p.scroll)
        .into_iter()
        .filter_map(|s| Some(((s.x, s.y), p.page.target(s.link)?.clone(), None)));
    let spots: Vec<((u16, u16), Link, Option<usize>)> = tabs.chain(links).collect();
    if spots.is_empty() {
        state.info("No links on screen");
        return;
    }
    let labels = hint_labels(spots.len());
    let labels = (spots.iter().zip(labels))
        .map(|(((x, y), ..), label)| HintLabel {
            x: *x,
            y: *y,
            label,
        })
        .collect();
    let (links, tabs) = spots.into_iter().map(|(_, link, tab)| (link, tab)).unzip();
    state.overlay = Some(Overlay::Hints(Box::new(Hints {
        labels,
        links,
        tabs,
        typed: String::new(),
        browser,
        tab: false,
    })));
}

#[must_use]
pub fn on_hints_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Hints(hints)) = &mut state.overlay else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Backspace => {
            hints.typed.pop();
            return Vec::new();
        }
        KeyCode::Char(c) if c.is_ascii_lowercase() => hints.typed.push(c),
        // A capital follows the link in a new tab.
        KeyCode::Char(c) if c.is_ascii_uppercase() => {
            hints.typed.push(c.to_ascii_lowercase());
            hints.tab = true;
        }
        _ => {
            state.overlay = None;
            return Vec::new();
        }
    }
    let matching: Vec<(&HintLabel, (&Link, &Option<usize>))> = hints
        .labels
        .iter()
        .zip(hints.links.iter().zip(&hints.tabs))
        .filter(|(h, _)| h.label.starts_with(&hints.typed))
        .collect();
    match matching.as_slice() {
        [] => {
            state.overlay = None;
            state.info("No link has those letters");
            Vec::new()
        }
        [(hint, (link, page_tab))] if hint.label == hints.typed => {
            let (link, page_tab) = ((*link).clone(), **page_tab);
            let (browser, tab) = (hints.browser, hints.tab);
            state.overlay = None;
            match link {
                // The page's own tab: switched to, not gone to.
                Link::Url(_) if !browser && !tab && page_tab.is_some() => {
                    switch_tab(state, page_tab.unwrap_or(1))
                }
                Link::Url(url) if browser => state.go(Target::External(url).into()),
                Link::Url(url) if tab => state.open_tab(Dest::from_url(&url)),
                link => state.follow(&link),
            }
        }
        _ => Vec::new(),
    }
}

impl Hints {
    /// Labels still matching what's typed.
    pub fn shown(&self) -> Vec<HintLabel> {
        self.labels
            .iter()
            .filter(|l| l.label.starts_with(&self.typed))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hint_labels_are_short_and_unique() {
        assert_eq!(hint_labels(3), ["a", "s", "d"]);
        let many = hint_labels(30);
        assert_eq!(many.len(), 30);
        assert_eq!(
            &many[..4],
            ["aa", "as", "sa", "ss"],
            "first letters spread out"
        );
        assert!(many.iter().all(|l| l.len() == 2));
        let mut sorted = many;
        sorted.dedup();
        assert_eq!(sorted.len(), 30);
    }
}
