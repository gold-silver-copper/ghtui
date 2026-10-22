//! Status bar and banners: flat filled bars with left- and right-aligned
//! content.

#![deny(clippy::arithmetic_side_effects)]

use crate::api::model::ChecksState;
use crate::theme::Bg;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::Widget;

use crate::ui::{Ctx, PAD_X, cols, fill, inset, key_hints, page, render_split, row, text};

const BAR: Bg = Bg::Container;

/// Transient message shown at the left of the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notice {
    Info(String),
    Error(String),
}

pub struct StatusBar<'a> {
    pub ctx: Ctx<'a>,
    /// What's loading, if anything, and the spinner's frame.
    pub busy: Option<&'a str>,
    pub spinner: &'a str,
    pub notice: Option<&'a Notice>,
    /// `(label, remaining, limit)` for the tightest rate-limit bucket.
    pub rate_limit: Option<(&'a str, u64, u64)>,
    /// How what's on screen is shown, when there's a choice (a diff by
    /// syntax or by lines).
    pub view: Option<&'a str>,
    /// `(keys, what they do)` here, most useful first; shown when there's no
    /// notice, as many as fit.
    pub hints: &'a [(String, String)],
}

impl Widget for StatusBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, BAR);
        let mut left = Vec::new();
        if let Some(busy) = self.busy {
            left.push(Span::styled(
                format!("{} {busy}", self.spinner),
                theme.accent(BAR),
            ));
            left.push(Span::styled("   ", theme.body(BAR)));
        }
        match self.notice {
            Some(Notice::Info(msg)) => left.push(Span::styled(msg.as_str(), theme.body(BAR))),
            Some(Notice::Error(msg)) => {
                let failed = self.ctx.icons.checks(ChecksState::Failing);
                left.push(Span::styled(format!("{failed} {msg}"), theme.error(BAR)));
            }
            None => {}
        }

        let mut right = Vec::new();
        if let Some(view) = self.view {
            right.push(Span::styled(view, theme.meta(BAR)));
            right.push(Span::styled("   ", theme.body(BAR)));
        }
        if let Some((label, remaining, limit)) = self.rate_limit {
            let low = remaining.saturating_mul(10) < limit;
            let style = if low {
                theme.error(BAR)
            } else {
                theme.meta(BAR)
            };
            right.push(Span::styled(format!("{label} {remaining}/{limit}"), style));
            right.push(Span::styled("   ", theme.body(BAR)));
        }
        if self.notice.is_none() {
            // As many hints as fit, whole.
            let used = text::spans_width(right.iter().chain(&left));
            let mut room = usize::from(area.width.saturating_sub(2 * PAD_X))
                .saturating_sub(used.saturating_add(1));
            for (i, (keys, what)) in self.hints.iter().enumerate() {
                let gap = if i == 0 { 1 } else { 4 };
                let w = text::width(keys).saturating_add(text::width(what));
                let Some(rest) = room.checked_sub(w.saturating_add(gap)) else { break };
                room = rest;
                if i > 0 {
                    left.push(Span::styled("   ", theme.body(BAR)));
                }
                left.extend(key_hints(theme, BAR, &[(keys.as_str(), what.as_str())]));
            }
        }
        while right
            .last()
            .is_some_and(|s: &Span<'_>| s.content.trim().is_empty())
        {
            right.pop();
        }
        render_split(inset(area, PAD_X, 0), buf, &left, &right, 1);
    }
}

/// A one-line inline banner (errors, notices) on a tonal container.
pub struct Banner<'a> {
    pub ctx: Ctx<'a>,
    pub text: &'a str,
    pub hint: Option<&'a str>,
    pub error: bool,
}

impl Banner<'_> {
    /// The text wrapped to `width`, beside the hint.
    fn lines(&self, width: u16) -> Vec<String> {
        let hint = self.hint.map_or(0, text::width);
        let room = usize::from(width.saturating_sub(2 * PAD_X + 2)).saturating_sub(hint);
        page::wrap(self.text, room)
    }

    /// Rows the banner takes at `width` (at most `max`).
    pub fn height(&self, width: u16, max: u16) -> u16 {
        cols(self.lines(width).len()).clamp(1, max.max(1))
    }
}

impl Widget for Banner<'_> {
    /// The text wraps over the area's rows (cut short on the last); the
    /// hint sits on the first.
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let bg = if self.error {
            Bg::ErrorContainer
        } else {
            Bg::SecondaryContainer
        };
        fill(buf, area, theme, bg);
        let style: Style = theme.fill(bg);
        let hint = self.hint.unwrap_or_default();
        let mut lines = self.lines(area.width);
        let rows = usize::from(area.height);
        if lines.len() > rows && rows > 0 {
            let rest = lines.split_off(rows.saturating_sub(1)).join(" ");
            let room = usize::from(area.width.saturating_sub(2 * PAD_X + 2))
                .saturating_sub(text::width(hint));
            lines.push(text::truncate(&rest, room));
        }
        let inner = inset(area, PAD_X, 0);
        for (i, (line, y)) in lines.into_iter().zip(area.top()..area.bottom()).enumerate() {
            let hint = if i == 0 { hint } else { "" };
            let right = [Span::styled(hint, style.add_modifier(Modifier::BOLD))];
            render_split(row(inner, y), buf, &[Span::styled(line, style)], &right, 1);
        }
    }
}
