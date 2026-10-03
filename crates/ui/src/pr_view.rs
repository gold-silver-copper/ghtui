//! Pull request overview: title, status chips, metadata and description.

use ghtui_api::model::{PrDetail, PrRef};
use ghtui_theme::{Bg, Fg};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::bars::Banner;
use crate::{Ctx, PAD_X, PAD_Y, chips, fill, text, time};

pub struct PrView<'a> {
    pub ctx: Ctx<'a>,
    pub pr: &'a PrRef,
    pub detail: Option<&'a PrDetail>,
    pub loading: bool,
    pub error: Option<&'a str>,
    pub scroll: u16,
    pub bg: Bg,
    /// Keys shown in the footer hint, e.g. "<Enter>" and "o".
    pub diff_key: &'a str,
    pub browser_key: &'a str,
}

impl PrView<'_> {
    /// Content lines at a given inner width (excluding padding).
    pub fn lines(&self, width: u16) -> Vec<Line<'static>> {
        let ctx = self.ctx;
        let theme = ctx.theme;
        let bg = self.bg;
        let width = usize::from(width.max(1));
        let Some(detail) = self.detail else {
            let msg = if self.loading {
                format!("Loading {}…", self.pr)
            } else {
                format!("Couldn't load {}.", self.pr)
            };
            return vec![Line::from(Span::styled(msg, theme.meta(bg)))];
        };
        let s = &detail.summary;
        let mut lines = Vec::new();

        let number = format!("  #{}", s.pr.number);
        let title_room = width.saturating_sub(text::width(&number)).max(1);
        let title_lines = text::wrap(&s.title, title_room);
        let last = title_lines.len().saturating_sub(1);
        for (i, line) in title_lines.into_iter().enumerate() {
            let mut spans = vec![Span::styled(line, theme.title(bg))];
            if i == last {
                spans.push(Span::styled(number.clone(), theme.meta(bg)));
            }
            lines.push(Line::from(spans));
        }
        lines.push(Line::default());

        // Status chips, wrapped onto as many lines as needed.
        let mut groups: Vec<Vec<Span<'static>>> = vec![chips::pr_state(ctx, s.state, bg)];
        if let Some(review) = s.review {
            groups.push(chips::review(ctx, review, bg));
        }
        if let Some(checks) = s.checks {
            groups.push(chips::checks(ctx, checks, bg));
        }
        if let Some(conflicts) = chips::mergeable(ctx, detail.mergeable, bg) {
            groups.push(conflicts);
        }
        for label in &detail.labels {
            groups.push(chips::label(ctx, label, bg));
        }
        let mut current: Vec<Span<'static>> = Vec::new();
        let mut used = 0;
        for group in groups {
            let w = chips::chip_width(&group);
            if used > 0 && used + 1 + w > width {
                lines.push(Line::from(std::mem::take(&mut current)));
                used = 0;
            }
            if used > 0 {
                current.push(Span::styled(" ", theme.body(bg)));
                used += 1;
            }
            current.extend(group);
            used += w;
        }
        lines.push(Line::from(current));
        lines.push(Line::default());

        let head = match &detail.head_repo {
            Some(repo) if *repo != s.pr.repo.to_string() => {
                let owner = repo.split('/').next().unwrap_or(repo);
                format!("{owner}:{}", detail.head_ref)
            }
            _ => detail.head_ref.clone(),
        };
        let meta = |t: String| Span::styled(t, theme.meta(bg));
        let strong = |t: String| Span::styled(t, theme.body(bg).add_modifier(Modifier::BOLD));
        lines.push(Line::from(vec![
            strong(s.author.clone()),
            meta(" wants to merge ".into()),
            strong(head),
            meta(" into ".into()),
            strong(detail.base_ref.clone()),
        ]));
        lines.push(Line::from(meta(format!(
            "Opened {} · updated {}",
            time::ago_iso(&detail.created_at, ctx.now),
            time::ago_iso(&s.updated_at, ctx.now)
        ))));
        lines.push(Line::from(vec![
            Span::styled(
                format!("+{}", s.additions),
                theme.style(Fg::DiffAddedSign, bg),
            ),
            Span::styled(" ", theme.body(bg)),
            Span::styled(
                format!("−{}", s.deletions),
                theme.style(Fg::DiffRemovedSign, bg),
            ),
            meta(format!(
                " · {} file{} changed · {} comment{} · head {}",
                detail.changed_files,
                plural(detail.changed_files),
                s.comments,
                plural(s.comments),
                short_sha(&detail.head_oid),
            )),
        ]));
        lines.push(Line::default());

        if detail.body.trim().is_empty() {
            lines.push(Line::from(Span::styled(
                "No description provided.",
                theme.meta(bg).add_modifier(Modifier::ITALIC),
            )));
        } else {
            for line in text::wrap(&detail.body.replace("\r\n", "\n"), width) {
                lines.push(Line::from(Span::styled(line, theme.body(bg))));
            }
        }
        lines.push(Line::default());
        let key =
            |k: &str| Span::styled(k.to_owned(), theme.accent(bg).add_modifier(Modifier::BOLD));
        lines.push(Line::from(vec![
            meta("Press ".into()),
            key(self.diff_key),
            meta(" to view the diff, or ".into()),
            key(self.browser_key),
            meta(format!(" to open on GitHub {}", ctx.icons.external())),
        ]));
        lines
    }

    /// Largest useful scroll offset for a content area of `area` size.
    pub fn max_scroll(&self, area: Rect) -> u16 {
        let inner = content_area(area, self.error.is_some());
        let total = self.lines(inner.width).len() as u16;
        total.saturating_sub(inner.height)
    }
}

fn plural(n: u64) -> &'static str {
    if n == 1 { "" } else { "s" }
}

fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

/// The text area inside the padding, below the banner if there is one.
fn content_area(area: Rect, banner: bool) -> Rect {
    let top = if banner { 1 + PAD_Y } else { PAD_Y };
    Rect {
        x: area.x + PAD_X,
        y: area.y + top.min(area.height),
        width: area.width.saturating_sub(2 * PAD_X),
        height: area.height.saturating_sub(top + PAD_Y),
    }
}

impl Widget for PrView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, self.bg);
        if let Some(error) = self.error {
            let banner_area = Rect { height: 1, ..area };
            let text = format!("Couldn't refresh: {error}");
            Banner {
                ctx: self.ctx,
                text: &text,
                hint: Some("r retry"),
                error: true,
            }
            .render(banner_area, buf);
        }
        let inner = content_area(area, self.error.is_some());
        let lines = self.lines(inner.width);
        let scroll = usize::from(self.scroll.min(self.max_scroll(area)));
        for (i, line) in lines
            .into_iter()
            .skip(scroll)
            .take(usize::from(inner.height))
            .enumerate()
        {
            line.render(
                Rect {
                    y: inner.y + i as u16,
                    height: 1,
                    ..inner
                },
                buf,
            );
        }
    }
}
