//! The view: a pure function of [`State`] (plus the current time).

use ghtui_theme::Bg;
use ghtui_ui::bars::{Banner, StatusBar, TopBar};
use ghtui_ui::overlays::{Help, Palette};
use ghtui_ui::page::PageView;
use ghtui_ui::{Ctx, PAD_X, PAD_Y, fill};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::Widget;

use ghtui_ui::diff_view::{DiffView, Keys};
use ghtui_ui::file_tree::{FileTree, TREE_BG};
use ghtui_ui::review_sheets::{ComposeSheet, SubmitSheet};

use crate::diff_screen::{self, DiffScreen, Pane};
use crate::keymap::{Action, format_sequence};
use crate::state::{Overlay, Screen, State};

/// The single content pane is focused, so it sits one tone above surface.
const PANE: Bg = Bg::ContainerLow;

pub fn view(state: &State, frame: &mut Frame, now: u64) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    render(state, area, buf, now);
}

pub fn render(state: &State, area: Rect, buf: &mut Buffer, now: u64) {
    let ctx = state.ctx(now);
    if area.height < 3 || area.width < 10 {
        fill(buf, area, ctx.theme, Bg::Surface);
        return;
    }
    let top = Rect { height: 1, ..area };
    let status = Rect {
        y: area.bottom() - 1,
        height: 1,
        ..area
    };
    let content = Rect {
        y: area.y + 1,
        height: area.height - 2,
        ..area
    };

    let tabs = state.tabs();
    TopBar {
        ctx,
        tabs: &tabs,
        active: tabs.len() - 1,
        login: state.viewer.as_deref(),
    }
    .render(top, buf);

    match state.screen() {
        Screen::Page(p) => {
            fill(buf, content, ctx.theme, PANE);
            // Pages are at most MAX_WIDTH wide, centered.
            let width = (state.page_width() + 2 * PAD_X).min(content.width);
            let area = Rect {
                x: content.x + (content.width - width) / 2,
                y: content.y + PAD_Y.min(content.height),
                width,
                height: content.height.saturating_sub(2 * PAD_Y),
            };
            PageView {
                ctx,
                page: &p.page,
                cursor: p.cursor,
                scroll: p.scroll,
            }
            .render(area, buf);
        }
        Screen::Diff(screen) => render_diff(state, ctx, content, buf, screen),
    }
    separate_collapsed_tones(ctx, content, buf);

    let busy = state.busy();
    let pending = format_sequence(&state.pending);
    let rate_limit = state
        .rate_limits
        .tightest()
        .map(|(_, b)| ("API", b.remaining, b.limit));
    StatusBar {
        ctx,
        busy: busy.as_deref(),
        notice: state.notice.as_ref(),
        pending_keys: &pending,
        rate_limit,
    }
    .render(status, buf);

    match &state.overlay {
        Some(Overlay::Help) => {
            let entries = state.help_entries();
            Help {
                ctx,
                entries: &entries,
            }
            .render(area, buf);
        }
        Some(Overlay::Palette(palette)) => {
            let input = palette.input.lines().join("");
            let items: Vec<_> = state
                .palette_commands(&input)
                .into_iter()
                .map(|(_, item)| item)
                .collect();
            Palette {
                ctx,
                prompt: ":",
                input: &palette.input,
                items: &items,
                selected: palette.selected,
            }
            .render(area, buf);
        }
        Some(Overlay::FindFile(finder)) => {
            let input = finder.input.lines().join("");
            let items: Vec<_> = state
                .finder_items(&input)
                .into_iter()
                .map(|(_, item)| item)
                .collect();
            Palette {
                ctx,
                prompt: "Find file",
                input: &finder.input,
                items: &items,
                selected: finder.selected,
            }
            .render(area, buf);
        }
        Some(Overlay::Compose(compose)) => {
            let title = compose.title();
            let note = match &compose.target {
                crate::review::ComposeTarget::File {
                    reason: Some(reason),
                    ..
                } => Some(reason.as_str()),
                _ => None,
            };
            ComposeSheet {
                ctx,
                title: &title,
                input: &compose.input,
                preview: compose
                    .preview
                    .as_ref()
                    .map(|p| (p.start_line, p.original.as_slice(), p.suggested.as_slice())),
                note,
                error: compose.error.as_deref(),
                sending: compose.sending,
                save: match compose.target {
                    crate::review::ComposeTarget::Reply { .. } => "post reply",
                    crate::review::ComposeTarget::Conversation { .. } => "post comment",
                    _ => "add to review",
                },
            }
            .render(
                Rect {
                    height: area.height.saturating_sub(1),
                    ..area
                },
                buf,
            );
        }
        Some(Overlay::Submit(dialog)) => {
            let (pending, rejected) = match state.screen() {
                Screen::Diff(screen) => state.diffs.get(&screen.pr).map_or((0, 0), |d| {
                    (
                        d.review.pending.len(),
                        d.review
                            .pending
                            .iter()
                            .filter(|p| p.error.is_some())
                            .count(),
                    )
                }),
                _ => (0, 0),
            };
            SubmitSheet {
                ctx,
                event: dialog.event,
                input: &dialog.input,
                pending,
                rejected,
                error: dialog.error.as_deref(),
                sending: dialog.sending,
            }
            .render(area, buf);
        }
        Some(Overlay::Commits(picker)) => {
            let items: Vec<_> = picker.items.iter().map(|(_, item)| item.clone()).collect();
            Palette {
                ctx,
                prompt: "Commits",
                input: &picker.input,
                items: &items,
                selected: picker.selected,
            }
            .render(area, buf);
        }
        Some(Overlay::Links(picker)) => {
            let items: Vec<_> = picker.items.iter().map(|(_, item)| item.clone()).collect();
            Palette {
                ctx,
                prompt: "Open",
                input: &picker.input,
                items: &items,
                selected: picker.selected,
            }
            .render(area, buf);
        }
        Some(Overlay::Prompt(prompt)) => {
            fill(buf, status, ctx.theme, Bg::Container);
            let row = Rect {
                x: status.x + PAD_X,
                width: status.width.saturating_sub(2 * PAD_X),
                ..status
            };
            let label = prompt.label();
            Span::styled(label, ctx.theme.accent(Bg::Container)).render(row, buf);
            let w = label.chars().count() as u16;
            prompt.input.render(
                Rect {
                    x: row.x + w,
                    width: row.width.saturating_sub(w),
                    ..row
                },
                buf,
            );
        }
        Some(Overlay::Search(input)) => {
            // The prompt replaces the status bar.
            fill(buf, status, ctx.theme, Bg::Container);
            let row = Rect {
                x: status.x + PAD_X,
                width: status.width.saturating_sub(2 * PAD_X),
                ..status
            };
            Span::styled("/", ctx.theme.accent(Bg::Container)).render(row, buf);
            input.render(
                Rect {
                    x: row.x + 1,
                    width: row.width.saturating_sub(1),
                    ..row
                },
                buf,
            );
        }
        None => {}
    }
}

fn first_key(state: &State, action: Action) -> String {
    state
        .keymap
        .keys_for(action)
        .into_iter()
        .next()
        .unwrap_or_else(|| format!(":{}", action.name()))
}

fn render_diff(state: &State, ctx: Ctx<'_>, content: Rect, buf: &mut Buffer, screen: &DiffScreen) {
    let theme = ctx.theme;
    let lay = diff_screen::layout(content, screen.tree_visible);
    fill(buf, content, theme, Bg::Surface);
    let Some(diff) = state.diffs.get(&screen.pr) else {
        return;
    };
    let mut area = lay.diff;
    if let Some(err) = &diff.error {
        let text = format!("Couldn't load the diff: {err}");
        Banner {
            ctx,
            text: &text,
            hint: Some("r retry"),
            error: true,
        }
        .render(Rect { height: 1, ..area }, buf);
        area.y += 1;
        area.height = area.height.saturating_sub(1);
    }
    let message_area = Rect {
        x: area.x + PAD_X,
        y: area.y + PAD_Y.min(area.height),
        width: area.width.saturating_sub(2 * PAD_X),
        height: 1,
    };
    if !diff.listed {
        if let Some(progress) = &diff.progress {
            Span::styled(
                ghtui_ui::text::truncate(progress, usize::from(message_area.width)),
                theme.meta(Bg::Surface),
            )
            .render(message_area, buf);
        }
        return;
    }
    if diff.doc.is_empty() {
        Span::styled("No changed files.", theme.meta(Bg::Surface)).render(message_area, buf);
        return;
    }
    let key = |a| first_key(state, a);
    let jump = key(Action::JumpMove);
    let (show, expand, viewed, reply, resolve, delete, file_comment) = (
        key(Action::Open),
        key(Action::ExpandContext),
        key(Action::ToggleViewed),
        key(Action::ReplyThread),
        key(Action::ResolveThread),
        key(Action::DeleteDraft),
        key(Action::FileComment),
    );
    DiffView {
        ctx,
        doc: &diff.doc,
        cursor: screen.cursor,
        top: screen.top,
        keys: Keys {
            show: &show,
            jump: &jump,
            expand: &expand,
            viewed: &viewed,
            reply: &reply,
            resolve: &resolve,
            delete: &delete,
            file_comment: &file_comment,
        },
        selection: screen.selection.map(|anchor| (anchor, screen.cursor)),
    }
    .render(area, buf);

    if let Some(tree) = lay.tree {
        FileTree {
            ctx,
            doc: &diff.doc,
            rows: &diff.tree,
            selected: screen.tree_selected,
            scroll: screen.tree_scroll,
            focused: screen.focus == Pane::Tree,
        }
        .render(tree, buf);
        // Tone separates the panes unless the colors quantized together.
        if theme.tones_collapse(TREE_BG, Bg::Surface) {
            let x = tree.right().saturating_sub(1);
            for y in tree.top()..tree.bottom() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol("│").set_style(theme.separator(TREE_BG));
                }
            }
        }
    }
}

/// Where the pane and the bars quantize to the same color (256-color
/// terminals), tone can't separate them; draw outline-variant rules in the
/// pane's padding rows instead.
fn separate_collapsed_tones(ctx: Ctx<'_>, content: Rect, buf: &mut Buffer) {
    if !ctx.theme.tones_collapse(Bg::Container, PANE) || content.height < 2 {
        return;
    }
    let style = ctx.theme.separator(PANE);
    for y in [content.y, content.bottom() - 1] {
        // Leave banners alone.
        let row_is_pane = buf
            .cell((content.x, y))
            .is_some_and(|c| c.bg == ctx.theme.bg_color(PANE) && c.symbol() == " ");
        if !row_is_pane {
            continue;
        }
        for x in content.left()..content.right() {
            if let Some(cell) = buf.cell_mut((x, y))
                && cell.symbol() == " "
            {
                cell.set_symbol("─").set_style(style);
            }
        }
    }
}
