//! ghtui's own pages: Home, and what's kept offline.

use super::{Family, Route, Target, Url};
use crate::browse::{Build, Width};
use crate::chrome::Chrome;
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Page {
    Home,
    /// The writes queued for GitHub and the repositories kept offline:
    /// ghtui's own page, with none on GitHub.
    Offline,
}

pub use Page::*;

impl Family for Page {
    fn url(&self) -> String {
        // The Offline page has none on GitHub: its link is Home's.
        format!("{}/", pages::url::base())
    }

    fn title(&self) -> String {
        match self {
            Home => "Home",
            Offline => "Offline",
        }
        .to_owned()
    }

    fn up(&self, _: &State) -> Option<Route> {
        match self {
            Home => None,
            Offline => Some(Route::App(Home)),
        }
    }

    /// Home's are its sections'.
    fn needs(&self, state: &State) -> Vec<DataKey> {
        match self {
            Home => state.home_needs(),
            Offline => Vec::new(),
        }
    }

    fn width(&self) -> Width {
        match self {
            Home => Width::Lists,
            Offline => Width::Reading,
        }
    }

    fn chrome(&self, _: &State, c: &mut Chrome) {
        c.crumb(&self.title(), None);
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        match self {
            Home => {
                let sections: Vec<pages::HomeSection<'_>> = (b.state.home.iter())
                    .map(|s| pages::HomeSection {
                        title: &s.title,
                        search: s.search.as_ref().map(|q| pages::SectionSearch {
                            kind: q.kind,
                            query: &q.query,
                            rows: q.rows,
                            results: b.get(&q.key()),
                        }),
                    })
                    .collect();
                pages::home(page, &sections, b.cx.icons, b.cx.now);
            }
            Offline => crate::offline::build(b.state, page, b.cx.now),
        }
    }
}

pub(super) fn parse(url: &Url<'_>) -> Option<Target> {
    matches!(url.segments, [] | ["dashboard"]).then_some(Target::Page(Route::App(Home)))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    proptest::strategy::Just(Route::App(Home))
}
