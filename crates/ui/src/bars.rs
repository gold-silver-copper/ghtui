//! Top bar (title and tabs) and status bar: flat filled bars on
//! surface-container with left- and right-aligned content.

use ghtui_theme::Bg;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::{Ctx, PAD_X, chips, fill, text};

const BAR: Bg = Bg::Container;

/// Renders `left` and `right` on one padded line, truncating the left side
/// first when they don't fit.
fn render_split(area: Rect, buf: &mut Buffer, left: Vec<Span<'_>>, right: Vec<Span<'_>>) {
    let inner = Rect {
        x: area.x + PAD_X.min(area.width / 2),
        width: area.width.saturating_sub(2 * PAD_X),
        ..area
    };
    let right_width = right.iter().map(Span::width).sum::<usize>() as u16;
    let right_width = right_width.min(inner.width);
    let left_area = Rect {
        width: inner.width.saturating_sub(right_width + 1),
        ..inner
    };
    let right_area = Rect {
        x: inner.right() - right_width,
        width: right_width,
        ..inner
    };
    Line::from(left).render(left_area, buf);
    Line::from(right).render(right_area, buf);
}

pub struct TopBar<'a> {
    pub ctx: Ctx<'a>,
    pub tabs: &'a [String],
    pub active: usize,
    /// Signed-in user, if known.
    pub login: Option<&'a str>,
}

impl Widget for TopBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, BAR);
        let mut left = vec![
            Span::styled("ghtui", theme.accent(BAR).add_modifier(Modifier::BOLD)),
            Span::styled("  ", theme.body(BAR)),
        ];
        for (i, tab) in self.tabs.iter().enumerate() {
            if i == self.active {
                left.extend(chips::chip(
                    self.ctx,
                    tab.clone(),
                    theme
                        .fill(Bg::SecondaryContainer)
                        .add_modifier(Modifier::BOLD),
                    BAR,
                ));
            } else {
                left.push(Span::styled(format!(" {tab} "), theme.meta(BAR)));
            }
            left.push(Span::styled(" ", theme.body(BAR)));
        }
        let right = match self.login {
            Some(login) => vec![Span::styled(format!("@{login}"), theme.meta(BAR))],
            None => Vec::new(),
        };
        render_split(area, buf, left, right);
    }
}

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
    /// Keys typed so far in a multi-key sequence.
    pub pending_keys: &'a str,
    /// `(label, remaining, limit)` for the tightest rate-limit bucket.
    pub rate_limit: Option<(&'a str, u64, u64)>,
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
            Some(Notice::Info(msg)) => left.push(Span::styled(msg.clone(), theme.body(BAR))),
            Some(Notice::Error(msg)) => left.push(Span::styled(
                format!(
                    "{} {msg}",
                    if self.ctx.icons.nerd_font {
                        "\u{f467}"
                    } else {
                        "✗"
                    }
                ),
                theme.error(BAR),
            )),
            None => {}
        }

        let mut right = Vec::new();
        if !self.pending_keys.is_empty() {
            right.push(Span::styled(
                self.pending_keys.to_owned(),
                theme.accent(BAR).add_modifier(Modifier::BOLD),
            ));
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
            let right_w: usize = right.iter().map(Span::width).sum();
            let mut room = usize::from(area.width.saturating_sub(2 * PAD_X))
                .saturating_sub(right_w + 1 + left.iter().map(Span::width).sum::<usize>());
            for (i, (keys, what)) in self.hints.iter().enumerate() {
                let gap = if i == 0 { 0 } else { 3 };
                let w = gap + text::width(keys) + 1 + text::width(what);
                if w > room {
                    break;
                }
                room -= w;
                if gap > 0 {
                    left.push(Span::styled("   ", theme.body(BAR)));
                }
                left.push(Span::styled(
                    keys.clone(),
                    theme.accent(BAR).add_modifier(Modifier::BOLD),
                ));
                left.push(Span::styled(format!(" {what}"), theme.meta(BAR)));
            }
        }
        while right
            .last()
            .is_some_and(|s: &Span<'_>| s.content.trim().is_empty())
        {
            right.pop();
        }
        render_split(area, buf, left, right);
    }
}

/// A one-line inline banner (errors, notices) on a tonal container.
pub struct Banner<'a> {
    pub ctx: Ctx<'a>,
    pub text: &'a str,
    pub hint: Option<&'a str>,
    pub error: bool,
}

impl Widget for Banner<'_> {
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
        let room = (area.width as usize).saturating_sub(2 * PAD_X as usize + text::width(hint) + 2);
        let left = vec![Span::styled(text::truncate(self.text, room), style)];
        let right = vec![Span::styled(
            hint.to_owned(),
            style.add_modifier(Modifier::BOLD),
        )];
        render_split(area, buf, left, right);
    }
}
