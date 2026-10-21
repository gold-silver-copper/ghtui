//! The comment composer and the review submit dialog.

#![deny(clippy::arithmetic_side_effects)]

use crate::api::model::ReviewEvent;
use crate::theme::{Bg, DiffBg, Fg, Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui_textarea::TextArea;
use std::fmt::Display;

use crate::ui::{Ctx, PAD_Y, centered, chips, cols, fill, fit_spans, idx, key_hints, padded, text};

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
    /// What an `@` or `#` being typed can be, and the one selected.
    pub completions: Option<(&'a [String], usize)>,
}

/// Rows a suggestion's preview of `lines` lines takes.
fn preview_rows(lines: Option<usize>) -> u16 {
    lines.map_or(0, |n| cols(n).saturating_add(2)).min(12)
}

/// Rows the composer wants, with a preview of `preview` lines and a note.
pub fn compose_rows(preview: Option<usize>, note: bool) -> u16 {
    (2 * PAD_Y + 2 + INPUT_ROWS + 2)
        .saturating_add(preview_rows(preview))
        .saturating_add(u16::from(note))
}

impl Widget for ComposeSheet<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let preview_rows = preview_rows(
            self.preview
                .map(|(_, o, s)| o.len().saturating_add(s.len())),
        );
        let inner = sheet(theme, area, buf);
        let row = |y| row(inner, y);
        let mut y = inner.y;
        say(self.title, theme.title(SHEET), row(y), buf);
        y = y.saturating_add(1);
        if let Some(note) = self.note {
            say(note, theme.meta(SHEET), row(y), buf);
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
        let completions = self.completions.zip(cursor_in(theme, input_area, buf));
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
                            &text::expand_tabs(line.as_str()),
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
                ("ctrl-g", "saved replies"),
            ],
        );
        if let Some(((choices, selected), at)) = completions {
            completion_list(theme, choices, selected, at, buf);
        }
    }
}

/// Where the text box in `area` shows its cursor: the cell in its
/// cursor's style (the box keeps where it's scrolled to itself).
fn cursor_in(theme: &Theme, area: Rect, buf: &Buffer) -> Option<(u16, u16)> {
    let cursor = theme.fill(Bg::Primary).bg;
    area.rows()
        .flat_map(Rect::columns)
        .find(|p| buf.cell(*p).is_some_and(|c| Some(c.bg) == cursor))
        .map(|p| (p.x, p.y))
}

/// The comment box's completion list, just under the cursor at `at` (or
/// over it, without room below).
fn completion_list(
    theme: &Theme,
    choices: &[String],
    selected: usize,
    (x, y): (u16, u16),
    buf: &mut Buffer,
) {
    const LIST: Bg = Bg::ContainerHighest;
    let screen = buf.area;
    let widest = choices.iter().map(|c| text::width(c)).max().unwrap_or(0);
    let width = cols(widest.saturating_add(2)).min(56).min(screen.width);
    let height = cols(choices.len());
    let below = y.saturating_add(1);
    let top = if below.saturating_add(height) <= screen.bottom() {
        below
    } else {
        y.saturating_sub(height)
    };
    let left = x.min(screen.right().saturating_sub(width));
    for (i, choice) in choices.iter().enumerate() {
        let bg = if i == selected {
            Bg::SecondaryContainer
        } else {
            LIST
        };
        let row = Rect {
            x: left,
            y: top.saturating_add(cols(i)),
            width,
            height: 1,
        };
        fill(buf, row, theme, bg);
        let shown = text::truncate(choice, usize::from(width.saturating_sub(2)));
        Span::styled(format!(" {shown}"), theme.body(bg)).render(row, buf);
    }
}

/// Fills a sheet's `area`, and gives the room inside its padding.
fn sheet(theme: &Theme, area: Rect, buf: &mut Buffer) -> Rect {
    fill(buf, area, theme, SHEET);
    padded(area)
}

/// Line `y` of a sheet's `inner` room.
fn row(inner: Rect, y: u16) -> Rect {
    Rect {
        y,
        height: 1,
        ..inner
    }
}

/// Writes `text` on one line of `area`, cut to fit.
fn say(text: &str, style: Style, area: Rect, buf: &mut Buffer) {
    Span::styled(text::fit(text, usize::from(area.width)), style).render(area, buf);
}

/// A row of buttons, each `(label, chosen)`: filled for the chosen one,
/// tonal for the others.
fn buttons(ctx: Ctx<'_>, labels: impl IntoIterator<Item = (impl Display, bool)>) -> Line<'static> {
    let theme = ctx.theme;
    let mut buttons = Vec::new();
    for (label, chosen) in labels {
        let style = if chosen {
            theme.fill(Bg::Primary).add_modifier(Modifier::BOLD)
        } else {
            theme.fill(Bg::SecondaryContainer)
        };
        buttons.extend(chips::chip(ctx, format!(" {label} "), style, SHEET));
        buttons.push(Span::styled("  ", theme.body(SHEET)));
    }
    Line::from(buttons)
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
        (None, Some(error)) => vec![Span::styled(error.to_owned(), theme.error(SHEET))],
        (None, None) => key_hints(theme, SHEET, hints),
    };
    Line::from(fit_spans(&spans, area.width)).render(area, buf);
}

pub struct SubmitSheet<'a> {
    pub ctx: Ctx<'a>,
    pub event: ReviewEvent,
    pub input: &'a TextArea<'static>,
    pub pending: usize,
    /// Comments already in your pending review on GitHub; `None` until
    /// GitHub says.
    pub on_github: Option<u64>,
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
        let inner = sheet(theme, area, buf);
        let row = |y| row(inner, y);
        let mut y = inner.y;
        Span::styled("Submit review", theme.title(SHEET)).render(row(y), buf);
        y = y.saturating_add(1);
        let s = if self.pending == 1 { "" } else { "s" };
        let pending = match (self.pending, self.rejected) {
            (0, _) => "No pending comments here.".to_owned(),
            (n, 0) => format!("{n} pending comment{s} will be added."),
            (n, r) => format!(
                "{n} pending comment{s}, {r} previously rejected (fix or delete those first)."
            ),
        };
        say(&pending, theme.meta(SHEET), row(y), buf);
        // What's on GitHub is published too, wherever it was written.
        let on_github = match self.on_github {
            None => Span::styled(
                "Checking GitHub for your pending review…",
                theme.meta(SHEET),
            ),
            Some(0) => Span::raw(""),
            Some(n) => Span::styled(
                if n == 1 {
                    "Your pending review on GitHub holds 1 comment: it's published too.".to_owned()
                } else {
                    format!(
                        "Your pending review on GitHub holds {n} comments: they're published too."
                    )
                },
                theme.error(SHEET),
            ),
        };
        Line::from(fit_spans(&[on_github], inner.width)).render(row(y.saturating_add(1)), buf);
        y = y.saturating_add(2);

        let events = [
            (ReviewEvent::Comment, "Comment"),
            (ReviewEvent::Approve, "Approve"),
            (ReviewEvent::RequestChanges, "Request changes"),
        ];
        let events = events.map(|(event, label)| (label, event == self.event));
        buttons(ctx, events).render(row(y), buf);
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
        let inner = sheet(theme, area, buf);
        let row = |y| row(inner, y);
        let mut y = inner.y;
        say(self.title, theme.title(SHEET), row(y), buf);
        y = y.saturating_add(2);
        for (fact, problem) in self.facts {
            let style = if *problem {
                theme.error(SHEET)
            } else {
                theme.meta(SHEET)
            };
            say(fact, style, row(y), buf);
            y = y.saturating_add(1);
        }
        if !self.facts.is_empty() {
            y = y.saturating_add(1);
        }
        let choices = self.choices.iter().enumerate();
        buttons(ctx, choices.map(|(i, label)| (label, i == self.selected))).render(row(y), buf);
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
            &[("⇥", "choose"), ("↵", "do it"), ("esc", "back")]
        } else {
            &[("↵", "do it"), ("esc", "back")]
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
