//! Copying links.

use super::*;

impl State {
    /// The web address under the cursor: in the word under it, else the
    /// selected row's link.
    pub fn link_under_cursor(&self) -> Option<String> {
        let in_word = |code: &str, col: Option<usize>| {
            let words = text::words(code);
            let word = text::word_at(&words, col.unwrap_or(0))?;
            text::url_at(code, &word.bytes).map(str::to_owned)
        };
        match self.screen() {
            Screen::Page(p) => (p.code_line())
                .and_then(|(_, code, _)| in_word(&code, p.word))
                .or_else(|| p.selected_link().and_then(Link::url).map(str::to_owned)),
            Screen::Diff(screen) => {
                let code = self.diff()?.doc.code_at(screen.cursor, screen.half)?;
                in_word(code, screen.word)
            }
        }
    }

    /// The link of the selection, or of the page.
    pub fn here_url(&self) -> String {
        // Lines selected in a file: their range.
        if let Screen::Page(p) = self.screen()
            && let Route::Blob {
                repo, rev, path, ..
            } = &p.route
            && let (Some(a), Some(b)) = (
                p.range.and_then(|i| p.line_of(i)),
                p.selected.and_then(|i| p.line_of(i)),
            )
        {
            let route = Route::Blob {
                repo: repo.clone(),
                rev: rev.clone(),
                path: path.clone(),
                lines: Some((a.min(b), a.max(b))),
            };
            return route.url();
        }
        let link = match self.screen() {
            Screen::Page(p) => p.selected_link().and_then(Link::url),
            Screen::Diff(d) => match self.diffs.get(&d.of) {
                Some(diff) => return crate::diff_screen::here_url(d, diff),
                None => None,
            },
        };
        link.map_or_else(|| crate::tabs::url(self.screen()), str::to_owned)
    }
}

/// `y`: the selection's link, or the page's.
#[must_use]
pub fn copy_link(state: &mut State) -> Vec<Cmd> {
    let url = state.here_url();
    state.info(format!("Copied {url}"));
    vec![Cmd::Copy(url)]
}
