//! The view: a pure function of [`State`] (plus the current time).

use ghtui_theme::Bg;
use ghtui_ui::bars::{Banner, StatusBar};
use ghtui_ui::chrome::{Header, KeyPanel, SearchPanel, TabBar, TitleBar, header_layout};
use ghtui_ui::overlays::Palette;
use ghtui_ui::page::PageView;
use ghtui_ui::{Ctx, PAD_X, PAD_Y, fill};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use ghtui_ui::diff_view::{DiffView, Keys};
use ghtui_ui::file_tree::{FileTree, TREE_BG};
use ghtui_ui::review_sheets::{ComposeSheet, SubmitSheet};

use crate::diff_screen::{self, DiffScreen, Pane};
use crate::keymap::Action;
use crate::state::{Overlay, Screen, State};

/// The single content pane is focused, so it sits one tone above surface.
const PANE: Bg = Bg::ContainerLow;

pub fn view(state: &State, frame: &mut Frame, now: u64) {
    let area = frame.area();
    let buf = frame.buffer_mut();
    let ctx = state.ctx(now);
    if area.height < 3 || area.width < 10 {
        fill(buf, area, ctx.theme, Bg::Surface);
        return;
    }
    let lay = state.layout();
    let chrome = state.chrome();
    let status = lay.status;

    // Header: where you are, the search field, you.
    let crumbs: Vec<_> = chrome.crumbs.iter().map(|(c, _)| c.clone()).collect();
    let right: Vec<_> = chrome.right.iter().map(|(t, _)| t.clone()).collect();
    let search_input = match &state.overlay {
        Some(Overlay::Search(sb)) => Some(&sb.input),
        _ => None,
    };
    let placeholder = format!("Type {} to search", state.first_key(Action::Search));
    Header {
        ctx,
        crumbs: &crumbs,
        right: &right,
        input: search_input,
        placeholder: &placeholder,
    }
    .render(lay.header, buf);
    if let (Some(rect), Some(pr)) = (lay.title, &chrome.title) {
        TitleBar {
            ctx,
            line: pr_title(state, pr),
        }
        .render(rect, buf);
    }
    if let Some(rect) = lay.tabs {
        let tabs: Vec<_> = chrome.tabs.iter().map(|(t, _)| t.clone()).collect();
        TabBar {
            ctx,
            tabs: &tabs,
            active: chrome.active,
        }
        .render(rect, buf);
    }

    let content = lay.content;
    match state.screen() {
        Screen::Page(p) => {
            fill(buf, content, ctx.theme, PANE);
            let hints = match &state.overlay {
                Some(Overlay::Hints(h)) => h.shown(),
                _ => Vec::new(),
            };
            PageView {
                ctx,
                page: &p.page,
                scroll: p.scroll,
                selected: p.selected,
                hints: &hints,
            }
            .render(state.page_area(), buf);
        }
        Screen::Diff(screen) => render_diff(state, ctx, content, buf, screen),
    }
    separate_collapsed_tones(ctx, content, buf);

    if let (Some(rect), Some(problem)) = (lay.problem, state.problems.values().next()) {
        Banner {
            ctx,
            text: problem,
            hint: None,
            error: true,
        }
        .render(rect, buf);
    }

    let busy = state.busy();
    let rate_limit = state
        .rate_limits
        .tightest()
        .map(|(_, b)| ("API", b.remaining, b.limit));
    StatusBar {
        ctx,
        busy: busy.as_deref(),
        spinner: crate::state::SPINNER
            .get(state.spinner % crate::state::SPINNER.len())
            .unwrap_or(&""),
        notice: state.notice.as_ref(),
        rate_limit,
        hints: &state.key_hints(),
    }
    .render(status, buf);

    match &state.overlay {
        Some(Overlay::Picker(p)) => {
            let items: Vec<_> = state
                .picker_rows(p)
                .into_iter()
                .map(|(item, _)| item)
                .collect();
            Palette {
                ctx,
                prompt: p.title(),
                input: &p.input,
                items: &items,
                selected: p.selected,
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
            let pending = state.diff().map_or(&[][..], |d| &d.inputs().review.pending);
            let rejected = pending.iter().filter(|p| p.error.is_some()).count();
            SubmitSheet {
                ctx,
                event: dialog.event,
                input: &dialog.input,
                pending: pending.len(),
                rejected,
                error: dialog.error.as_deref(),
                sending: dialog.sending,
            }
            .render(area, buf);
        }
        Some(Overlay::Menu(menu)) => {
            let (rows, selected) = state.menu_rows(menu);
            let title = match &menu.filter {
                Some(query) => format!("Here you can: /{query}▏"),
                None => "Here you can".to_owned(),
            };
            KeyPanel {
                ctx,
                title: &title,
                rows: &rows,
                selected,
                position: (menu.selected + 1, menu.shown().len()),
            }
            .render(area, buf);
        }
        Some(Overlay::Search(sb)) => {
            let rows: Vec<_> = state.suggestions(sb).into_iter().map(|(r, _)| r).collect();
            let field = header_layout(lay.header, &crumbs, &right).search;
            SearchPanel {
                ctx,
                rows: &rows,
                selected: sb.selected,
                field,
            }
            .render(area, buf);
        }
        Some(Overlay::DiffSearch(input)) => {
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
        Some(Overlay::Hints(_)) | None => {}
    }
}

fn render_diff(state: &State, ctx: Ctx<'_>, content: Rect, buf: &mut Buffer, screen: &DiffScreen) {
    let theme = ctx.theme;
    let lay = diff_screen::layout(content, screen.prefs.tree_visible);
    fill(buf, content, theme, Bg::Surface);
    let Some(diff) = state.diffs.get(&screen.of) else {
        return;
    };
    let mut area = lay.diff;
    if let Some(err) = &diff.error {
        let text = format!("Couldn't load the diff: {err}");
        let retry = format!("{} retry", state.first_key(Action::Refresh));
        let banner = Banner {
            ctx,
            text: &text,
            hint: Some(&retry),
            error: true,
        };
        // Long git errors wrap, up to a third of the pane.
        let height = banner.height(area.width, area.height / 3);
        banner.render(Rect { height, ..area }, buf);
        area.y += height;
        area.height = area.height.saturating_sub(height);
    }
    let message_area = Rect {
        x: area.x + PAD_X,
        y: area.y + PAD_Y.min(area.height),
        width: area.width.saturating_sub(2 * PAD_X),
        height: 1,
    };
    if !diff.listed() {
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
    let key = |a| state.first_key(a);
    let jump = key(Action::JumpMove);
    let (show, expand, viewed, reply, resolve, delete, file_comment) = (
        key(Action::Open),
        key(Action::ExpandContext),
        key(Action::ToggleViewed),
        key(Action::Comment),
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

/// A pull request's sticky title: state, title, number.
fn pr_title<'a>(state: &'a State, pr: &ghtui_api::model::PrRef) -> Line<'a> {
    let theme = &state.theme;
    let bar = Bg::Container;
    let Some(d) = state.prs.get(pr).and_then(|r| r.data.as_ref()) else {
        return Line::from(Span::styled(pr.to_string(), theme.title(bar)));
    };
    let s = &d.summary;
    let (text, bg) = match s.state {
        ghtui_api::model::PrState::Open => ("Open", Bg::SuccessContainer),
        ghtui_api::model::PrState::Draft => ("Draft", Bg::SecondaryContainer),
        ghtui_api::model::PrState::Merged => ("Merged", Bg::TertiaryContainer),
        ghtui_api::model::PrState::Closed => ("Closed", Bg::ErrorContainer),
    };
    Line::from(vec![
        Span::styled(
            format!(" {} {text} ", state.icons.pr_state(s.state)),
            theme.fill(bg),
        ),
        Span::styled("  ", theme.body(bar)),
        Span::styled(s.title.clone(), theme.title(bar)),
        Span::styled(format!("  #{}", pr.number), theme.meta(bar)),
    ])
}
