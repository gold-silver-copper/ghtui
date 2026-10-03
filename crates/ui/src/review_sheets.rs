//! The comment composer (a bottom sheet) and the review submit dialog.

use ghtui_api::model::ReviewEvent;
use ghtui_theme::{Bg, DiffBg, Fg};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use ratatui_textarea::TextArea;

use crate::{Ctx, PAD_Y, centered, chips, fill, padded, text};

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
    pub is_reply: bool,
}

impl Widget for ComposeSheet<'_> {
    fn render(self, screen: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let preview_rows = self
            .preview
            .map_or(0, |(_, o, s)| (o.len() + s.len()) as u16 + 2)
            .min(12);
        let height =
            (2 * PAD_Y + 2 + INPUT_ROWS + 2 + preview_rows + u16::from(self.note.is_some()))
                .min(screen.height.saturating_sub(2));
        let area = Rect {
            x: screen.x + 1,
            y: screen.bottom().saturating_sub(height + 1),
            width: screen.width.saturating_sub(2),
            height,
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
        y += 1;
        if let Some(note) = self.note {
            Span::styled(
                text::truncate(note, usize::from(inner.width)),
                theme.meta(SHEET),
            )
            .render(row(y), buf);
            y += 1;
        }
        y += 1;
        let input_rows = INPUT_ROWS.min(inner.bottom().saturating_sub(y + 2 + preview_rows));
        fill(
            buf,
            Rect {
                y,
                height: input_rows,
                ..inner
            },
            theme,
            SHEET,
        );
        self.input.render(
            Rect {
                y,
                height: input_rows,
                ..inner
            },
            buf,
        );
        y += input_rows + 1;

        if let Some((start, original, suggested)) = self.preview {
            Span::styled("Preview of the suggestion", theme.meta(SHEET)).render(row(y), buf);
            y += 1;
            let lines = original
                .iter()
                .enumerate()
                .map(|(i, l)| (DiffBg::Removed, "-", start + i as u32, l))
                .chain(
                    suggested
                        .iter()
                        .enumerate()
                        .map(|(i, l)| (DiffBg::Added, "+", start + i as u32, l)),
                );
            for (diff_bg, sign, n, line) in lines {
                if y + 1 >= inner.bottom() {
                    break;
                }
                let bg = Bg::Diff(diff_bg);
                let r = row(y);
                fill(buf, r, theme, bg);
                let fg = if sign == "+" {
                    Fg::DiffAddedSign
                } else {
                    Fg::DiffRemovedSign
                };
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
                y += 1;
            }
            y += 1;
        }

        let footer = row(inner.bottom().saturating_sub(1).max(y));
        let key = |k: &str| {
            Span::styled(
                k.to_owned(),
                theme.accent(SHEET).add_modifier(Modifier::BOLD),
            )
        };
        let meta = |t: &str| Span::styled(t.to_owned(), theme.meta(SHEET));
        let spans = if self.sending {
            vec![meta("Posting…")]
        } else if let Some(error) = self.error {
            vec![Span::styled(
                text::truncate(error, usize::from(footer.width)),
                theme.error(SHEET),
            )]
        } else {
            vec![
                key("<C-s>"),
                meta(if self.is_reply {
                    " post reply · "
                } else {
                    " add to review · "
                }),
                key("<C-e>"),
                meta(" open in $EDITOR · "),
                key("Esc"),
                meta(" cancel"),
            ]
        };
        Line::from(spans).render(footer, buf);
    }
}

pub struct SubmitSheet<'a> {
    pub ctx: Ctx<'a>,
    pub event: ReviewEvent,
    pub input: &'a TextArea<'static>,
    pub pending: usize,
    pub rejected: usize,
    pub error: Option<&'a str>,
    pub sending: bool,
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
        y += 1;
        let pending = match (self.pending, self.rejected) {
            (0, _) => "No pending comments.".to_owned(),
            (n, 0) => format!(
                "{n} pending comment{} will be added.",
                if n == 1 { "" } else { "s" }
            ),
            (n, r) => format!(
                "{n} pending comment{}, {r} previously rejected (fix or delete those first).",
                if n == 1 { "" } else { "s" }
            ),
        };
        Span::styled(pending, theme.meta(SHEET)).render(row(y), buf);
        y += 2;

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
        y += 2;

        let input_area = Rect {
            y,
            height: 3.min(inner.bottom().saturating_sub(y + 2)),
            ..inner
        };
        fill(buf, input_area, theme, SHEET);
        self.input.render(input_area, buf);

        let footer = row(inner.bottom().saturating_sub(1));
        let key = |k: &str| {
            Span::styled(
                k.to_owned(),
                theme.accent(SHEET).add_modifier(Modifier::BOLD),
            )
        };
        let meta = |t: &str| Span::styled(t.to_owned(), theme.meta(SHEET));
        let spans = if self.sending {
            vec![meta("Submitting…")]
        } else if let Some(error) = self.error {
            vec![Span::styled(
                text::truncate(error, usize::from(footer.width)),
                theme.error(SHEET),
            )]
        } else {
            vec![
                key("Tab"),
                meta(" choose · "),
                key("<C-s>"),
                meta(" submit · "),
                key("<C-e>"),
                meta(" $EDITOR · "),
                key("Esc"),
                meta(" cancel"),
            ]
        };
        Line::from(spans).render(footer, buf);
    }
}
