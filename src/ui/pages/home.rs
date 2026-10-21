//! Home: its sections.

use super::*;

/// One of Home's sections: the first rows of a search.
pub struct HomeSection<'a> {
    pub title: &'a str,
    /// Its search, or what's wrong with how it's written.
    pub search: Result<SectionSearch<'a>, &'a String>,
}

pub struct SectionSearch<'a> {
    pub kind: SearchKind,
    pub query: &'a str,
    pub rows: usize,
    pub results: Fetched<'a, SearchResults>,
}

/// The first rows of a search's results, the rest listed at `url`.
pub(super) struct Start<'a, T> {
    pub(super) list: &'a Results<T>,
    pub(super) rows: usize,
    pub(super) url: String,
}

impl<T> Rows<T> for Start<'_, T> {
    fn rows(&self) -> &[T] {
        self.list.items.get(..self.rows).unwrap_or(&self.list.items)
    }
    fn rest(&self) -> Rest {
        let shown = self.rows().len() as u64;
        let total = self.list.total.max(self.list.items.len() as u64);
        match total - shown {
            0 => Rest::All,
            n => Rest::At(n, self.url.clone()),
        }
    }
}

/// Home: each section a box of its search's first rows, titled with how
/// many of how many, ending in a row that opens the rest. A section that
/// fails, or is written wrong, says why in its box; the others show.
pub fn home(page: &mut Page, sections: &[HomeSection<'_>], icons: Icons, now: u64) {
    // Where GitHub's header has its inbox.
    let inbox = format!("{}/notifications", url::base());
    let inbox = button(page, "Notifications", inbox);
    page.line(vec![inbox]);
    page.blank();
    if sections.is_empty() {
        let none = "No sections. Add a search from its list (in the Space menu), or as [[home]] in config.toml.";
        empty_box(page, strong("Home"), none);
    }
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            page.blank();
        }
        page.anchor(format!("section-{i}"));
        let title = |count: Option<String>| {
            let mut t = vec![strong(section.title.to_owned())];
            t.extend(count.map(|c| meta(format!("  {c}"))));
            t
        };
        let failed = |page: &mut Page, why: &str| {
            page.box_top(title(None), Vec::new());
            section_note(page, i, why, Role::Error);
            page.box_bottom();
        };
        let search = match &section.search {
            Ok(search) => search,
            Err(why) => {
                failed(page, &format!("This section is written wrong: {why}."));
                continue;
            }
        };
        let loading = |page: &mut Page| loading_box(page, title(None));
        let what = section.title.to_lowercase();
        let Some(results) = search.results.show_with(page, &what, loading, failed) else {
            continue;
        };
        let url = url::search(search.kind, search.query);
        let noun = match search.kind {
            SearchKind::Repos => "repositories",
            SearchKind::Pulls => "pull requests",
            _ => "issues",
        };
        let empty = format!("No {noun} match “{}”.", search.query);
        let rows = search.rows;
        match results {
            SearchResults::Issues(list) => {
                let start = Start { list, rows, url };
                let title = title(Some(count_of(&start)));
                // Asked to review: a row opens straight to the review.
                let review = search.query.contains("review-requested:");
                let shown = (true, by_update(search.query));
                section_box(page, title, &start, (i, &empty), |page, it| {
                    issue_row(page, it, shown, icons, now, review);
                });
            }
            SearchResults::Repos(list) => {
                let start = Start { list, rows, url };
                let title = title(Some(count_of(&start)));
                section_box(page, title, &start, (i, &empty), |page, r| {
                    repo_row(page, r, now, true);
                });
            }
            _ => failed(page, "GitHub sent the wrong kind of results"),
        }
    }
}

/// A section's box of rows; with none, its `empty` note.
pub(super) fn section_box<T>(
    page: &mut Page,
    title: Vec<Seg>,
    start: &Start<'_, T>,
    (i, empty): (usize, &str),
    row: impl FnMut(&mut Page, &T),
) {
    page.box_top(title, Vec::new());
    if start.rows().is_empty() {
        section_note(page, i, empty, Role::Meta);
    }
    box_rows(page, start, row);
    page.box_bottom();
}

/// A row that's section `i`'s own (opening its list, or saying what's
/// wrong), so every section has a row to select and change it by.
pub(super) fn section_note(page: &mut Page, i: usize, text: &str, role: Role) {
    item(page, Link::Section(i), |page, link| {
        page.wrapped(vec![Seg::linked(text, role, link)], 0, Frame::Body);
    });
}
