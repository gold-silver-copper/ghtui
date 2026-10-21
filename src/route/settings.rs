//! A repository's Settings tab, at one of its pages ([`crate::settings`]).

use super::{Family, Route, Target, Url};
use crate::api::model::RepoId;
use crate::api::settings::SettingsPage;
use crate::browse::Build;
use crate::chrome::Section;
use crate::data::DataKey;
use crate::state::State;
use crate::ui::pages::url as links;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Page {
    pub repo: RepoId,
    pub page: SettingsPage,
}

impl Family for Page {
    fn url(&self) -> String {
        format!("{}/settings{}", links::repo(&self.repo), self.page.slug())
    }

    fn title(&self) -> String {
        let repo = &self.repo;
        match self.page {
            SettingsPage::Hook(id) => format!("{repo} · Webhook {id}"),
            page => format!("{repo} · Settings · {}", page.name()),
        }
    }

    fn repo(&self) -> Option<&RepoId> {
        Some(&self.repo)
    }

    fn up(&self, _: &State) -> Option<Route> {
        let repo = self.repo.clone();
        Some(match self.page {
            SettingsPage::Hook(_) => Route::Settings(Page {
                repo,
                page: SettingsPage::Hooks,
            }),
            _ => Route::code(repo),
        })
    }

    fn needs(&self) -> Vec<DataKey> {
        let repo = self.repo.clone();
        vec![
            DataKey::Repo(repo.clone()),
            DataKey::Settings(repo, self.page),
        ]
    }

    fn section(&self) -> Section {
        Section::Settings
    }

    fn build(&self, b: Build<'_>, page: &mut crate::ui::page::Page) {
        let settings = b.get(&DataKey::Settings(self.repo.clone(), self.page));
        crate::settings::build(b.state, page, &self.repo, self.page, settings, b.cx.now);
    }
}

/// The Settings tab's pages ghtui has; the rest are on GitHub.
pub(super) fn parse_repo(repo: &RepoId, rest: &[&str], _: &Url<'_>) -> Option<Target> {
    use SettingsPage as P;
    let ["settings", rest @ ..] = rest else {
        return None;
    };
    let page = match rest {
        [] => P::General,
        ["branches" | "branch_protection_rules" | "rules", ..] => P::Branches,
        ["access" | "collaboration"] => P::Access,
        ["hooks"] | ["hooks", "new"] => P::Hooks,
        ["hooks", id] => P::Hook(id.parse().ok().filter(|&n| n > 0)?),
        ["keys", ..] => P::Keys,
        _ => return None,
    };
    let repo = repo.clone();
    Some(Target::Page(Route::Settings(Page { repo, page })))
}

#[cfg(test)]
pub(crate) fn arb() -> impl proptest::strategy::Strategy<Value = Route> {
    use crate::route::arb::repo;
    use proptest::prelude::*;
    let page = prop_oneof![
        Just(SettingsPage::General),
        Just(SettingsPage::Branches),
        Just(SettingsPage::Access),
        Just(SettingsPage::Hooks),
        Just(SettingsPage::Keys),
        (1..u64::MAX).prop_map(SettingsPage::Hook),
    ];
    (repo(), page).prop_map(|(repo, page)| Route::Settings(Page { repo, page }))
}
