//! The comment composer (a bottom sheet) and the review submit dialog.

#![deny(clippy::arithmetic_side_effects)]

use ghtui_api::model::ReviewEvent;
use ghtui_theme::{Bg, DiffBg, Fg, Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui_textarea::TextArea;

use crate::{Ctx, PAD_Y, centered, chips, cols, fill, idx, key_hints, padded, text};

const SHEET: Bg = Bg::ContainerHigh;
const INPUT_ROWS: u16 = 8;

pub struct ComposeSheet<'a> {
    pub ctx: Ctx<'a>,
    pub title: &'a str,
    pub input: &'a TextArea<'static>,
    /// A suggestion: `(first line, original lines, suggested lines)`.
    pub preview: Option<(u32, &'a [String], &'a [String])>,
    pub note: Option<&'a str>,
    pub error: Option<&'a str>,
    pub sending: bool,
    /// What `ctrl-s` does: "post reply", "add to review"…
    pub save: &'a str,
}

impl Widget for ComposeSheet<'_> {
    fn render(self, screen: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let preview_rows = self
            .preview
            .map_or(0, |(_, o, s)| {
                cols(o.len().saturating_add(s.len())).saturating_add(2)
            })
            .min(12);
        let height = (2 * PAD_Y + 2 + INPUT_ROWS + 2)
            .saturating_add(preview_rows)
            .saturating_add(u16::from(self.note.is_some()))
            .min(screen.height.saturating_sub(2));
        let area = Rect {
            y: screen.bottom().saturating_sub(height.saturating_add(1)),
            height,
            ..screen
        };
        fill(buf, area, theme, SHEET);
        let inner = padded(area);
        let mut y = inner.y;
        let row = |y: u16| Rect {
            y,
            height: 1,
            ..inner
        };
        Span::styled(self.title.to_owned(), theme.title(SHEET)).render(row(y), buf);
        y = y.saturating_add(1);
        if let Some(note) = self.note {
            Span::styled(
                text::truncate(note, usize::from(inner.width)),
                theme.meta(SHEET),
            )
            .render(row(y), buf);
            y = y.saturating_add(1);
        }
        y = y.saturating_add(1);
        let input_rows = INPUT_ROWS.min(
            inner
                .bottom()
                .saturating_sub(y)
                .saturating_sub(preview_rows.saturating_add(2)),
        );
        let input_area = Rect {
            y,
            height: input_rows,
            ..inner
        };
        self.input.render(input_area, buf);
        y = y.saturating_add(input_rows).saturating_add(1);

        if let Some((start, original, suggested)) = self.preview {
            Span::styled("Preview of the suggestion", theme.meta(SHEET)).render(row(y), buf);
            y = y.saturating_add(1);
            let removed = (DiffBg::Removed, "-", Fg::DiffRemovedSign);
            let added = (DiffBg::Added, "+", Fg::DiffAddedSign);
            let lines = original.iter().enumerate().map(|(i, l)| (removed, i, l));
            let lines = lines.chain(suggested.iter().enumerate().map(|(i, l)| (added, i, l)));
            for ((diff_bg, sign, fg), i, line) in lines {
                if y.saturating_add(1) >= inner.bottom() {
                    break;
                }
                let n = start.saturating_add(idx(i));
                let bg = Bg::Diff(diff_bg);
                let r = row(y);
                fill(buf, r, theme, bg);
                Line::from(vec![
                    Span::styled(format!("{n:>4} "), theme.meta(bg)),
                    Span::styled(sign, theme.style(fg, bg).add_modifier(Modifier::BOLD)),
                    Span::styled(" ", theme.body(bg)),
                    Span::styled(
                        text::truncate(
                            &line.replace('\t', "    "),
                            usize::from(r.width).saturating_sub(8),
                        ),
                        theme.body(bg),
                    ),
                ])
                .render(r, buf);
                y = y.saturating_add(1);
            }
            y = y.saturating_add(1);
        }

        footer(
            theme,
            row(inner.bottom().saturating_sub(1).max(y)),
            buf,
            self.sending.then_some("Posting…"),
            self.error,
            &[
                ("ctrl-s", self.save),
                ("ctrl-e", "open in $EDITOR"),
                ("esc", "cancel"),
            ],
        );
    }
}

/// A sheet's last line: progress, the error, or its keys.
fn footer(
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
    busy: Option<&str>,
    error: Option<&str>,
    hints: &[(&str, &str)],
) {
    let spans = match (busy, error) {
        (Some(busy), _) => vec![Span::styled(busy.to_owned(), theme.meta(SHEET))],
        (None, Some(error)) => vec![Span::styled(
            text::truncate(error, usize::from(area.width)),
            theme.error(SHEET),
        )],
        (None, None) => key_hints(theme, SHEET, hints),
    };
    Line::from(spans).render(area, buf);
}

pub struct SubmitSheet<'a> {
    pub ctx: Ctx<'a>,
    pub event: ReviewEvent,
    pub input: &'a TextArea<'static>,
    pub pending: usize,
    pub rejected: usize,
    pub error: Option<&'a str>,
    pub sending: bool,
    /// Enter submits (opened to approve, from a pull request's page).
    pub quick: bool,
}

impl Widget for SubmitSheet<'_> {
    fn render(self, screen: Rect, buf: &mut Buffer) {
        let ctx = self.ctx;
        let theme = ctx.theme;
        let height = 2 * PAD_Y + 10;
        let top = screen.height.saturating_sub(height) / 2;
        let area = centered(screen, 76.min(screen.width.saturating_sub(4)), height, top);
        fill(buf, area, theme, SHEET);
        let inner = padded(area);
        let row = |y: u16| Rect {
            y,
            height: 1,
            ..inner
        };
        let mut y = inner.y;
        Span::styled("Submit review", theme.title(SHEET)).render(row(y), buf);
        y = y.saturating_add(1);
        let s = if self.pending == 1 { "" } else { "s" };
        let pending = match (self.pending, self.rejected) {
            (0, _) => "No pending comments.".to_owned(),
            (n, 0) => format!("{n} pending comment{s} will be added."),
            (n, r) => format!(
                "{n} pending comment{s}, {r} previously rejected (fix or delete those first)."
            ),
        };
        Span::styled(pending, theme.meta(SHEET)).render(row(y), buf);
        y = y.saturating_add(2);

        // Filled button for the chosen action, tonal for the others.
        let mut buttons: Vec<Span<'static>> = Vec::new();
        for (event, label) in [
            (ReviewEvent::Comment, "Comment"),
            (ReviewEvent::Approve, "Approve"),
            (ReviewEvent::RequestChanges, "Request changes"),
        ] {
            let style = if event == self.event {
                theme.fill(Bg::Primary).add_modifier(Modifier::BOLD)
            } else {
                theme.fill(Bg::SecondaryContainer)
            };
            buttons.extend(chips::chip(ctx, format!(" {label} "), style, SHEET));
            buttons.push(Span::styled("  ", theme.body(SHEET)));
        }
        Line::from(buttons).render(row(y), buf);
        y = y.saturating_add(2);

        let input_area = Rect {
            y,
            height: 3.min(inner.bottom().saturating_sub(y).saturating_sub(2)),
            ..inner
        };
        self.input.render(input_area, buf);

        footer(
            theme,
            row(inner.bottom().saturating_sub(1)),
            buf,
            self.sending.then_some("Submitting…"),
            self.error,
            if self.quick {
                &[
                    ("↵", "submit"),
                    ("⇥", "choose"),
                    ("ctrl-e", "$EDITOR"),
                    ("esc", "cancel"),
                ]
            } else {
                &[
                    ("⇥", "choose"),
                    ("ctrl-s", "submit"),
                    ("ctrl-e", "$EDITOR"),
                    ("esc", "cancel"),
                ]
            },
        );
    }
}

/// Asks before a change to GitHub: what it is, what's worth knowing first
/// (`facts`, each a problem or not), and the ways to do it, one chosen.
pub struct ConfirmSheet<'a> {
    pub ctx: Ctx<'a>,
    pub title: &'a str,
    pub facts: &'a [(String, bool)],
    pub choices: &'a [String],
    pub selected: usize,
    pub error: Option<&'a str>,
    /// What's happening while GitHub answers ("Merging…").
    pub sending: Option<&'a str>,
}

impl Widget for ConfirmSheet<'_> {
    fn render(self, screen: Rect, buf: &mut Buffer) {
        let ctx = self.ctx;
        let theme = ctx.theme;
        let width = 76.min(screen.width.saturating_sub(4));
        let error_rows: u16 = if self.error.is_some() { 3 } else { 0 };
        let height = (2 * PAD_Y + 6)
            .saturating_add(cols(self.facts.len()))
            .saturating_add(error_rows);
        let top = screen.height.saturating_sub(height) / 2;
        let area = centered(screen, width, height, top);
        fill(buf, area, theme, SHEET);
        let inner = padded(area);
        let row = |y: u16| Rect {
            y,
            height: 1,
            ..inner
        };
        let mut y = inner.y;
        let title = text::truncate(self.title, usize::from(inner.width));
        Span::styled(title, theme.title(SHEET)).render(row(y), buf);
        y = y.saturating_add(2);
        for (fact, problem) in self.facts {
            let style = if *problem {
                theme.error(SHEET)
            } else {
                theme.meta(SHEET)
            };
            let fact = text::truncate(fact, usize::from(inner.width));
            Span::styled(fact, style).render(row(y), buf);
            y = y.saturating_add(1);
        }
        if !self.facts.is_empty() {
            y = y.saturating_add(1);
        }
        // Filled button for the chosen way, tonal for the others.
        let mut buttons: Vec<Span<'static>> = Vec::new();
        for (i, label) in self.choices.iter().enumerate() {
            let style = if i == self.selected {
                theme.fill(Bg::Primary).add_modifier(Modifier::BOLD)
            } else {
                theme.fill(Bg::SecondaryContainer)
            };
            buttons.extend(chips::chip(ctx, format!(" {label} "), style, SHEET));
            buttons.push(Span::styled("  ", theme.body(SHEET)));
        }
        Line::from(buttons).render(row(y), buf);
        if let Some(error) = self.error {
            let area = Rect {
                y: y.saturating_add(2),
                height: error_rows,
                ..inner
            };
            ratatui::widgets::Paragraph::new(error.to_owned())
                .style(theme.error(SHEET))
                .wrap(ratatui::widgets::Wrap { trim: true })
                .render(area, buf);
        }
        let hints: &[(&str, &str)] = if self.choices.len() > 1 {
            &[("⇥", "choose"), ("↵", "do it"), ("esc", "cancel")]
        } else {
            &[("↵", "do it"), ("esc", "cancel")]
        };
        footer(
            theme,
            row(inner.bottom().saturating_sub(1)),
            buf,
            self.sending,
            None,
            hints,
        );
    }
}
