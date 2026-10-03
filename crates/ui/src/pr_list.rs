//! The inbox: review requests and my open pull requests as two-line rows.

use ghtui_api::model::{Inbox, PrState, PrSummary};
use ghtui_theme::{Bg, Fg};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::{Ctx, PAD_X, chips, fill, text, time};

/// A line-level item in the list.
#[derive(Debug, Clone, Copy)]
pub enum Row<'a> {
    Header {
        title: &'static str,
        shown: usize,
        total: u64,
    },
    Empty(&'static str),
    Pr {
        index: usize,
        pr: &'a PrSummary,
    },
    Spacer,
}

impl Row<'_> {
    pub fn height(&self) -> u16 {
        match self {
            Row::Pr { .. } => 2,
            _ => 1,
        }
    }
}

/// Rows for the inbox, review requests first. PR indices are positions in
/// [`flat_prs`].
pub fn rows(inbox: &Inbox) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    let mut index = 0;
    let sections = [
        (
            "Review requested",
            &inbox.review_requested,
            inbox.review_requested_total,
            "Nothing is waiting for your review.",
        ),
        (
            "Your pull requests",
            &inbox.authored,
            inbox.authored_total,
            "You have no open pull requests.",
        ),
    ];
    for (i, (title, prs, total, empty)) in sections.into_iter().enumerate() {
        if i > 0 {
            rows.push(Row::Spacer);
        }
        rows.push(Row::Header {
            title,
            shown: prs.len(),
            total,
        });
        if prs.is_empty() {
            rows.push(Row::Empty(empty));
        }
        for pr in prs {
            rows.push(Row::Pr { index, pr });
            index += 1;
        }
    }
    rows
}

/// PRs in display order; selection indexes into this.
pub fn flat_prs(inbox: &Inbox) -> Vec<&PrSummary> {
    inbox
        .review_requested
        .iter()
        .chain(&inbox.authored)
        .collect()
}

/// First line and height of the selected PR's row.
pub fn selected_lines(rows: &[Row<'_>], selected: usize) -> Option<(u16, u16)> {
    let mut line = 0;
    for row in rows {
        if let Row::Pr { index, .. } = row
            && *index == selected
        {
            return Some((line, row.height()));
        }
        line += row.height();
    }
    None
}

pub fn total_lines(rows: &[Row<'_>]) -> u16 {
    rows.iter().map(Row::height).sum()
}

/// Smallest scroll change that keeps the selected row fully visible, with
/// one row of context when possible.
pub fn scroll_to_selection(rows: &[Row<'_>], selected: usize, scroll: u16, viewport: u16) -> u16 {
    let Some((start, height)) = selected_lines(rows, selected) else {
        return 0;
    };
    let context = if viewport > 2 * height + 2 { 1 } else { 0 };
    let max_scroll = total_lines(rows).saturating_sub(viewport);
    let mut scroll = scroll.min(max_scroll);
    // Keep the section header visible when selecting the first PR under it.
    let top = start.saturating_sub(if start <= 2 { start } else { context });
    if top < scroll {
        scroll = top;
    }
    let bottom = start + height + context;
    if bottom > scroll + viewport {
        scroll = bottom.saturating_sub(viewport);
    }
    scroll.min(max_scroll)
}

pub struct PrList<'a> {
    pub ctx: Ctx<'a>,
    pub rows: &'a [Row<'a>],
    pub selected: Option<usize>,
    pub scroll: u16,
    pub bg: Bg,
    pub selected_bg: Bg,
}

impl Widget for PrList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, self.bg);
        let mut y = area.y as i32 - i32::from(self.scroll);
        for row in self.rows {
            let height = row.height() as i32;
            if y + height <= area.y as i32 {
                y += height;
                continue;
            }
            if y >= area.bottom() as i32 {
                break;
            }
            let selected = matches!(row, Row::Pr { index, .. } if Some(*index) == self.selected);
            let bg = if selected { self.selected_bg } else { self.bg };
            for dy in 0..height {
                let line_y = y + dy;
                if line_y < area.y as i32 || line_y >= area.bottom() as i32 {
                    continue;
                }
                let line_area = Rect {
                    x: area.x,
                    y: line_y as u16,
                    width: area.width,
                    height: 1,
                };
                if selected {
                    fill(buf, line_area, theme, bg);
                }
                let inner = Rect {
                    x: area.x + PAD_X,
                    width: area.width.saturating_sub(2 * PAD_X),
                    ..line_area
                };
                self.render_line(row, dy as u16, bg, inner, buf);
            }
            y += height;
        }
    }
}

impl PrList<'_> {
    fn render_line(&self, row: &Row<'_>, line: u16, bg: Bg, area: Rect, buf: &mut Buffer) {
        let ctx = self.ctx;
        let theme = ctx.theme;
        match row {
            Row::Header {
                title,
                shown,
                total,
            } => {
                // GitHub's total can lag the results; never show "3 of 2".
                let count = if *total > *shown as u64 {
                    format!("  {shown} of {total}, most recently updated")
                } else {
                    format!("  {shown}")
                };
                Line::from(vec![
                    Span::styled(*title, theme.title(bg)),
                    Span::styled(count, theme.meta(bg)),
                ])
                .render(area, buf)
            }
            Row::Empty(msg) => Span::styled(*msg, theme.meta(bg)).render(area, buf),
            Row::Spacer => {}
            Row::Pr { pr, .. } => {
                let (left, right) = if line == 0 {
                    self.first_line(pr, bg)
                } else {
                    self.second_line(pr, bg)
                };
                let right_width = right.iter().map(Span::width).sum::<usize>();
                let left_room = (area.width as usize).saturating_sub(right_width + 2);
                let left = fit(left, left_room);
                Line::from(left).render(area, buf);
                let right_width = (right_width as u16).min(area.width);
                Line::from(right).render(
                    Rect {
                        x: area.right() - right_width,
                        width: right_width,
                        ..area
                    },
                    buf,
                );
            }
        }
    }

    fn first_line<'s>(&self, pr: &'s PrSummary, bg: Bg) -> (Vec<Span<'s>>, Vec<Span<'s>>) {
        let ctx = self.ctx;
        let theme = ctx.theme;
        let state_fg = match pr.state {
            PrState::Open => Fg::Success,
            PrState::Draft => Fg::OnSurfaceVariant,
            PrState::Merged => Fg::Tertiary,
            PrState::Closed => Fg::Error,
        };
        let mut left = vec![
            Span::styled(ctx.icons.pr_state(pr.state), theme.style(state_fg, bg)),
            Span::styled(" ", theme.body(bg)),
        ];
        if pr.state == PrState::Draft {
            left.push(Span::styled("Draft ", theme.meta(bg)));
        }
        left.push(Span::styled(pr.title.as_str(), theme.title(bg)));

        let mut right = Vec::new();
        if let Some(review) = pr.review {
            right.extend(chips::review(ctx, review, bg));
            right.push(Span::styled(" ", theme.body(bg)));
        }
        if let Some(checks) = pr.checks {
            right.extend(chips::checks_short(ctx, checks, bg));
        }
        (left, right)
    }

    fn second_line<'s>(&self, pr: &'s PrSummary, bg: Bg) -> (Vec<Span<'s>>, Vec<Span<'s>>) {
        let ctx = self.ctx;
        let theme = ctx.theme;
        let left = vec![
            Span::styled("  ", theme.body(bg)),
            Span::styled(pr.pr.to_string(), theme.meta(bg)),
            Span::styled(
                format!(
                    " · {} · updated {}",
                    pr.author,
                    time::ago_iso(&pr.updated_at, ctx.now)
                ),
                theme.meta(bg),
            ),
        ];
        let mut right = vec![
            Span::styled(
                format!("+{}", pr.additions),
                theme.style(Fg::DiffAddedSign, bg),
            ),
            Span::styled(" ", theme.body(bg)),
            Span::styled(
                format!("−{}", pr.deletions),
                theme.style(Fg::DiffRemovedSign, bg),
            ),
        ];
        if pr.comments > 0 {
            right.push(Span::styled(
                format!(
                    "  {} comment{}",
                    pr.comments,
                    if pr.comments == 1 { "" } else { "s" }
                ),
                theme.meta(bg),
            ));
        }
        (left, right)
    }
}

/// Truncates the last span so the line fits in `room` columns.
fn fit(spans: Vec<Span<'_>>, room: usize) -> Vec<Span<'_>> {
    let mut used = 0;
    let mut out = Vec::new();
    for span in spans {
        let w = span.width();
        if used + w <= room {
            used += w;
            out.push(span);
            continue;
        }
        let rest = room.saturating_sub(used);
        if rest > 0 {
            let style = span.style;
            out.push(Span::styled(text::truncate(&span.content, rest), style));
        }
        break;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghtui_api::model::{PrRef, PrState};

    fn pr(n: u64) -> PrSummary {
        PrSummary {
            pr: PrRef::parse(&format!("o/r#{n}")).unwrap(),
            title: format!("PR {n}"),
            author: "a".into(),
            state: PrState::Open,
            updated_at: "2026-01-01T00:00:00Z".into(),
            additions: 1,
            deletions: 1,
            comments: 0,
            review: None,
            checks: None,
        }
    }

    fn inbox(requested: u64, authored: u64) -> Inbox {
        Inbox {
            review_requested: (1..=requested).map(pr).collect(),
            authored: (100..100 + authored).map(pr).collect(),
            ..Inbox::default()
        }
    }

    #[test]
    fn rows_have_headers_and_empty_states() {
        let inbox = inbox(0, 2);
        let rows = rows(&inbox);
        assert!(matches!(rows[0], Row::Header { shown: 0, .. }));
        assert!(matches!(rows[1], Row::Empty(_)));
        assert!(matches!(rows[2], Row::Spacer));
        assert!(matches!(rows[3], Row::Header { shown: 2, .. }));
        assert!(matches!(rows[4], Row::Pr { index: 0, .. }));
        assert_eq!(flat_prs(&inbox).len(), 2);
    }

    #[test]
    fn selection_scrolls_into_view() {
        let inbox = inbox(3, 30);
        let rows = rows(&inbox);
        let viewport = 10;
        let mut scroll = 0;
        for selected in 0..33 {
            scroll = scroll_to_selection(&rows, selected, scroll, viewport);
            let (start, height) = selected_lines(&rows, selected).unwrap();
            assert!(
                start >= scroll && start + height <= scroll + viewport,
                "{selected}"
            );
        }
        // And back up again.
        for selected in (0..33).rev() {
            scroll = scroll_to_selection(&rows, selected, scroll, viewport);
            let (start, height) = selected_lines(&rows, selected).unwrap();
            assert!(
                start >= scroll && start + height <= scroll + viewport,
                "{selected}"
            );
        }
        assert_eq!(scroll, 0, "first PR shows its section header");
    }
}
