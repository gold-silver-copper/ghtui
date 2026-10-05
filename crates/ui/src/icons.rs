//! Glyphs: Nerd Font when enabled, plain Unicode otherwise. Icons always
//! accompany text; they never carry meaning alone.

use ghtui_api::model::{ChecksState, PrState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Icons {
    pub nerd_font: bool,
}

impl Icons {
    pub fn pr_state(self, state: PrState) -> &'static str {
        match (self.nerd_font, state) {
            (true, PrState::Open) => "\u{f407}",
            (true, PrState::Draft) => "\u{f4dd}",
            (true, PrState::Merged) => "\u{f419}",
            (true, PrState::Closed) => "\u{f4dc}",
            // Arrows, unlike the checks' marks, and narrow in every locale.
            (false, PrState::Open) => "⇄",
            (false, PrState::Draft) => "⇢",
            (false, PrState::Merged) => "⇉",
            (false, PrState::Closed) => "⇸",
        }
    }

    pub fn checks(self, state: ChecksState) -> &'static str {
        match (self.nerd_font, state) {
            (true, ChecksState::Passing) => "\u{f42e}",
            (true, ChecksState::Failing) => "\u{f467}",
            (true, ChecksState::Pending) => "\u{f444}",
            (false, ChecksState::Passing) => "✓",
            (false, ChecksState::Failing) => "✗",
            (false, ChecksState::Pending) => "◔",
        }
    }

    pub fn folder(self) -> &'static str {
        if self.nerd_font { "\u{f07b}" } else { "▸" }
    }

    pub fn file(self) -> &'static str {
        if self.nerd_font { "\u{f15b}" } else { "·" }
    }

    pub fn comment(self) -> &'static str {
        // Not 💬: emoji are two columns wide and drawn inconsistently.
        if self.nerd_font { "\u{f41f}" } else { "❝" }
    }

    pub fn external(self) -> &'static str {
        if self.nerd_font { "\u{f465}" } else { "↗" }
    }

    /// Rounded chip ends (Powerline half circles), only with a Nerd Font.
    pub fn chip_caps(self) -> Option<(&'static str, &'static str)> {
        self.nerd_font.then_some(("\u{e0b6}", "\u{e0b4}"))
    }
}
