//! Tonal chips: short labels on filled container backgrounds.

use ghtui_api::model::{ChecksState, Label, Mergeable, PrState, ReviewDecision};
use ghtui_theme::{Bg, Fg};
use ratatui::style::Style;
use ratatui::text::Span;

use crate::Ctx;

/// A chip's spans: text padded by one cell, with rounded caps when the
/// Nerd Font is enabled. `parent` is the background the chip sits on.
pub fn chip<'a>(ctx: Ctx<'_>, text: impl Into<String>, style: Style, parent: Bg) -> Vec<Span<'a>> {
    let text = text.into();
    match (ctx.icons.chip_caps(), style.bg) {
        (Some((left, right)), Some(chip_bg)) => {
            let cap = Style::new().fg(chip_bg).bg(ctx.theme.bg_color(parent));
            vec![
                Span::styled(left, cap),
                Span::styled(text, style),
                Span::styled(right, cap),
            ]
        }
        _ => vec![Span::styled(format!(" {text} "), style)],
    }
}

/// Chip width in cells, matching [`chip`].
pub fn chip_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|s| s.width()).sum()
}

fn tonal(ctx: Ctx<'_>, bg: Bg) -> Style {
    ctx.theme.fill(bg)
}

pub fn pr_state<'a>(ctx: Ctx<'_>, state: PrState, parent: Bg) -> Vec<Span<'a>> {
    let icon = ctx.icons.pr_state(state);
    let (text, style) = match state {
        PrState::Open => ("Open", tonal(ctx, Bg::SuccessContainer)),
        PrState::Draft => (
            "Draft",
            ctx.theme.style(Fg::OnSurfaceVariant, Bg::ContainerHighest),
        ),
        PrState::Merged => ("Merged", tonal(ctx, Bg::TertiaryContainer)),
        PrState::Closed => ("Closed", tonal(ctx, Bg::ErrorContainer)),
    };
    chip(ctx, format!("{icon} {text}"), style, parent)
}

pub fn review<'a>(ctx: Ctx<'_>, decision: ReviewDecision, parent: Bg) -> Vec<Span<'a>> {
    let (text, bg) = match decision {
        ReviewDecision::Approved => ("Approved", Bg::SuccessContainer),
        ReviewDecision::ChangesRequested => ("Changes requested", Bg::ErrorContainer),
        ReviewDecision::ReviewRequired => ("Review required", Bg::SecondaryContainer),
    };
    chip(ctx, text, tonal(ctx, bg), parent)
}

pub fn checks<'a>(ctx: Ctx<'_>, state: ChecksState, parent: Bg) -> Vec<Span<'a>> {
    let (text, bg) = match state {
        ChecksState::Passing => ("Checks passing", Bg::SuccessContainer),
        ChecksState::Failing => ("Checks failing", Bg::ErrorContainer),
        ChecksState::Pending => ("Checks pending", Bg::SecondaryContainer),
    };
    chip(
        ctx,
        format!("{} {text}", ctx.icons.checks(state)),
        tonal(ctx, bg),
        parent,
    )
}

/// Compact checks chip for list rows.
pub fn checks_short<'a>(ctx: Ctx<'_>, state: ChecksState, parent: Bg) -> Vec<Span<'a>> {
    let (text, bg) = match state {
        ChecksState::Passing => ("passing", Bg::SuccessContainer),
        ChecksState::Failing => ("failing", Bg::ErrorContainer),
        ChecksState::Pending => ("pending", Bg::SecondaryContainer),
    };
    chip(
        ctx,
        format!("{} {text}", ctx.icons.checks(state)),
        tonal(ctx, bg),
        parent,
    )
}

pub fn mergeable<'a>(ctx: Ctx<'_>, mergeable: Mergeable, parent: Bg) -> Option<Vec<Span<'a>>> {
    (mergeable == Mergeable::Conflicting)
        .then(|| chip(ctx, "Conflicts", tonal(ctx, Bg::ErrorContainer), parent))
}

pub fn label<'a>(ctx: Ctx<'_>, label: &Label, parent: Bg) -> Vec<Span<'a>> {
    chip(
        ctx,
        label.name.clone(),
        ctx.theme.label_chip(&label.color),
        parent,
    )
}
