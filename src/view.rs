//! The view: a pure function of [`State`] (plus the current time).

use crate::theme::Bg;
use crate::ui::bars::{Banner, StatusBar};
use crate::ui::chrome::{Header, KeyPanel, SearchField, SearchPanel, TabBar, TitleBar};
use crate::ui::overlays::Palette;
use crate::ui::page::{Link, PageView, columns};
use crate::ui::{Ctx, PAD_X, PAD_Y, fill};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::ui::diff_view::{DiffView, Keys};
use crate::ui::file_tree::{FileTree, TREE_BG};
use crate::ui::review_sheets::{ComposeSheet, ConfirmSheet, SubmitSheet};

use crate::diff_screen::{self, DiffScreen, Pane};
use crate::keymap::Action;
use crate::review::Compose;
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
    let chrome = state.chrome();
    let lay = state.layout_of(&chrome);
    let status = lay.status;

    // Header: where you are, the search field, you.
    let crumbs: Vec<_> = chrome.crumbs.iter().map(|(c, _)| c.clone()).collect();
    let right: Vec<_> = chrome.right.iter().map(|(t, _)| t.clone()).collect();
    // A filter is typed into the list's own field, not the header's.
    let search_input = match &state.overlay {
        Some(Overlay::Search(sb)) if !state.search_field(sb).1 => Some(&sb.input),
        _ => None,
    };
    let placeholder = format!("Type {} to search", state.first_key(Action::SearchGithub));
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
    let composing = match &state.overlay {
        Some(Overlay::Compose(compose)) => Some(compose.as_ref()),
        _ => None,
    };
    let slot = match state.screen() {
        Screen::Page(p) => {
            fill(buf, content, ctx.theme, PANE);
            let hints = match &state.overlay {
                Some(Overlay::Hints(h)) => h.shown(),
                _ => Vec::new(),
            };
            let area = state.page_area();
            let page = p.page();
            // The composer takes the comment box's place, goes under the
            // comment it quotes, or takes the place of the comment it
            // edits, scrolled into view.
            let comment_box = composing.and_then(|c| {
                let (at, under) = match &c.target {
                    crate::review::ComposeTarget::Conversation {
                        quoting: Some(at), ..
                    } => (Some(at), true),
                    crate::review::ComposeTarget::Edit(e) => (Some(&e.at), false),
                    crate::review::ComposeTarget::Discussion(w) => w.place(),
                    _ => (None, false),
                };
                let comment = at.and_then(|at| {
                    page.items.iter().find(|it| {
                        matches!(page.target(it.link), Some(Link::Quote { at: was, .. }) if was == at)
                    })
                });
                let (item, under) = match comment {
                    Some(item) => (*item, under),
                    None => (*page.items.get(page.find(&Link::Comment, 0)?)?, false),
                };
                Some((usize::from(c.rows()), item, under))
            });
            let scroll = comment_box.map_or(p.scroll, |(rows, item, under)| {
                let at = if under { item.end } else { item.start };
                scroll_to(item.start, at, rows, p.scroll, area.height)
            });
            PageView {
                ctx,
                page,
                scroll,
                selected: p.selected,
                from: p.range,
                current: p.current_match(),
                hints: &hints,
                shift: p.shift,
                word: p
                    .word
                    .and_then(|_| p.word_cursor())
                    .map(|(at, _, w)| (at, w.cols)),
            }
            .render(area, buf);
            comment_box.map(|(_, item, under)| {
                let x = columns(page, area).main_x;
                let (at, replaced) = if under {
                    (item.end, 0)
                } else {
                    (item.start, item.end.saturating_sub(item.start))
                };
                Slot {
                    area: Rect {
                        x,
                        width: page.width.min(area.right().saturating_sub(x)),
                        ..area
                    },
                    offset: at.saturating_sub(scroll),
                    replaced,
                }
            })
        }
        Screen::Diff(screen) => render_diff(state, ctx, content, buf, screen, composing),
    };
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
        notice: state.notices.shown(),
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
            let completions: Option<Vec<String>> = crate::typing::completions(state)
                .map(|choices| choices.into_iter().map(|c| c.label).collect());
            let rows = compose.rows();
            // Where the comment will go, or (without room there) a strip
            // above the status bar.
            let sheet = match slot.filter(|s| s.area.height >= rows) {
                Some(slot) => slot.make_room(buf, rows),
                None => {
                    let height = rows.min(area.height.saturating_sub(3));
                    Rect {
                        y: area.bottom().saturating_sub(height.saturating_add(2)),
                        height,
                        ..area
                    }
                }
            };
            ComposeSheet {
                ctx,
                title: &title,
                input: &compose.input,
                preview: compose
                    .preview
                    .as_ref()
                    .map(|p| (p.start_line, p.original.as_slice(), p.suggested.as_slice())),
                note: compose.note(),
                error: compose.error.as_deref(),
                sending: compose.sending,
                save: match &compose.target {
                    crate::review::ComposeTarget::Reply { .. } => "post reply",
                    crate::review::ComposeTarget::Conversation { .. } => "post comment",
                    crate::review::ComposeTarget::Form(crate::route::Route::NewPr(_)) => {
                        "create pull request…"
                    }
                    crate::review::ComposeTarget::Form(_) => "create issue",
                    crate::review::ComposeTarget::Edit(_) => "save",
                    crate::review::ComposeTarget::Discussion(w) => w.save(),
                    crate::review::ComposeTarget::ProjectDraft(d) if d.draft.is_some() => "save",
                    crate::review::ComposeTarget::ProjectDraft(_) => "add draft",
                    _ => "add to review",
                },
                completions: (completions.as_deref()).map(|c| {
                    (
                        c,
                        compose.completing.selected.min(c.len().saturating_sub(1)),
                    )
                }),
            }
            .render(sheet, buf);
        }
        Some(Overlay::Submit(dialog)) => {
            let pending = dialog.drafts(state).unwrap_or_default();
            let rejected = pending.iter().filter(|p| p.error.is_some()).count();
            SubmitSheet {
                ctx,
                event: dialog.event,
                input: &dialog.input,
                pending: pending.len(),
                on_github: dialog.on_github,
                rejected,
                error: dialog.error.as_deref(),
                sending: dialog.sending,
                quick: dialog.quick,
            }
            .render(area, buf);
        }
        Some(Overlay::Confirm(confirm)) => {
            let choices: Vec<String> = confirm.choices.iter().map(|(l, _)| l.clone()).collect();
            let busy = confirm.busy();
            ConfirmSheet {
                ctx,
                title: &confirm.title,
                facts: &confirm.facts,
                choices: &choices,
                selected: confirm.selected,
                error: confirm.error.as_deref(),
                sending: busy.as_deref(),
            }
            .render(area, buf);
        }
        Some(Overlay::Ask(ask)) => {
            let choices: Vec<String> = ask.choices.iter().map(|(l, _)| l.clone()).collect();
            ConfirmSheet {
                ctx,
                title: &ask.title,
                facts: &ask.facts,
                choices: &choices,
                selected: ask.selected,
                error: ask.error.as_deref(),
                sending: ask.busy(),
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
                selected: Some(selected),
                position: (menu.selected + 1, menu.shown().len()),
            }
            .render(area, buf);
        }
        Some(Overlay::Search(sb)) => {
            let rows: Vec<_> = state.suggestions(sb).into_iter().map(|(r, _)| r).collect();
            let (field, in_place) = state.search_field(sb);
            if in_place {
                SearchField {
                    ctx,
                    input: &sb.input,
                }
                .render(field, buf);
            }
            SearchPanel {
                ctx,
                rows: &rows,
                selected: sb.selected,
                field,
            }
            .render(area, buf);
        }
        Some(Overlay::Find(input)) => {
            // The prompt replaces the status bar.
            fill(buf, status, ctx.theme, Bg::Container);
            let row = Rect {
                x: status.x + PAD_X,
                width: status.width.saturating_sub(2 * PAD_X),
                ..status
            };
            let prompt = if state.backwards { "?" } else { "/" };
            Span::styled(prompt, ctx.theme.accent(Bg::Container)).render(row, buf);
            input.render(
                Rect {
                    x: row.x + 1,
                    width: row.width.saturating_sub(1),
                    ..row
                },
                buf,
            );
        }
        // After the first keys of a sequence: what can follow.
        None => {
            if let Some((typed, rows)) = state.which_key() {
                KeyPanel {
                    ctx,
                    title: &typed,
                    rows: &rows,
                    selected: None,
                    position: (0, rows.len()),
                }
                .render(area, buf);
            }
        }
        Some(Overlay::Hints(_)) => {}
    }
}

/// Draws the diff; with `composing`, scrolled to show the composer under
/// the row it's about, whose slot it returns.
fn render_diff(
    state: &State,
    ctx: Ctx<'_>,
    content: Rect,
    buf: &mut Buffer,
    screen: &DiffScreen,
    composing: Option<&Compose>,
) -> Option<Slot> {
    let theme = ctx.theme;
    fill(buf, content, theme, Bg::Surface);
    let diff = state.diffs.get(&screen.of)?;
    let lay = diff_screen::layout(content, &screen.prefs, &diff.tree);
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
                crate::ui::text::truncate(progress, usize::from(message_area.width)),
                theme.meta(Bg::Surface),
            )
            .render(message_area, buf);
        }
        return None;
    }
    if diff.doc.is_empty() {
        Span::styled("No changed files.", theme.meta(Bg::Surface)).render(message_area, buf);
        return None;
    }
    let key = |a| state.first_key(a);
    let jump = key(Action::JumpMove);
    let (show, expand, viewed, reply, resolve, delete, file_comment) = (
        key(Action::Open),
        key(Action::ExpandContext),
        key(Action::ToggleViewed),
        key(Action::Comment),
        key(Action::ResolveThread),
        key(Action::Delete),
        key(Action::FileComment),
    );
    // The composer goes under its row, which stays clear of the sticky
    // header.
    let doc = &diff.doc;
    let anchor = composing.map(|c| crate::review::compose_anchor(doc, screen, &c.target));
    let mut top = doc.to_global(screen.top);
    let slot = composing.zip(anchor).map(|(c, anchor)| {
        let row = doc.to_global(anchor);
        let first = row.saturating_sub(usize::from(anchor.row > 0));
        let rows = usize::from(c.rows());
        top = scroll_to(first, row.saturating_add(1), rows, top, area.height);
        Slot {
            area,
            offset: row.saturating_add(1).saturating_sub(top),
            replaced: 0,
        }
    });
    DiffView {
        ctx,
        doc,
        cursor: screen.cursor,
        half: screen.half,
        top: doc.to_pos(top),
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
        shift: screen.shift,
        word: screen.word,
    }
    .render(area, buf);
    if let Some(Overlay::Hints(hints)) = &state.overlay {
        crate::ui::page::draw_hints(theme, &hints.shown(), area, buf);
    }

    if let Some(tree) = lay.tree {
        FileTree {
            ctx,
            doc: &diff.doc,
            rows: &crate::ui::file_tree::unfolded(&diff.tree, &screen.folded),
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
    slot
}

/// The scroll that shows rows `start..start + rows` while row `first`
/// stays in view: `first` wins when they don't fit together.
fn scroll_to(first: usize, start: usize, rows: usize, scroll: usize, height: u16) -> usize {
    let lowest = start
        .saturating_add(rows)
        .saturating_sub(usize::from(height));
    scroll.max(lowest).min(first)
}

/// Where the composer goes: `offset` rows into `area`, in place of
/// `replaced` rows.
#[derive(Clone, Copy)]
struct Slot {
    area: Rect,
    offset: usize,
    replaced: usize,
}

impl Slot {
    /// Moves what's below the slot down to make room for `rows` rows
    /// (kept on screen, over what's above if they must), and returns them.
    fn make_room(self, buf: &mut Buffer, rows: u16) -> Rect {
        let Slot { area, .. } = self;
        let offset = u16::try_from(self.offset).unwrap_or(u16::MAX);
        let replaced = u16::try_from(self.replaced).unwrap_or(u16::MAX);
        let y = area
            .y
            .saturating_add(offset)
            .min(area.bottom().saturating_sub(rows));
        let shift = rows.saturating_sub(replaced);
        for to in (y.saturating_add(rows)..area.bottom()).rev() {
            let from = to.saturating_sub(shift);
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell((x, from)).cloned()
                    && let Some(dest) = buf.cell_mut((x, to))
                {
                    *dest = cell;
                }
            }
        }
        Rect {
            y,
            height: rows,
            ..area
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
fn pr_title<'a>(state: &'a State, pr: &crate::api::model::PrRef) -> Line<'a> {
    let theme = &state.theme;
    let bar = Bg::Container;
    let Some(d) = state.prs.get(pr).and_then(|r| r.data.as_ref()) else {
        return Line::from(Span::styled(pr.to_string(), theme.title(bar)));
    };
    let (text, bg) = crate::ui::pages::state_label(crate::api::browse::Status::Pr(d.state));
    Line::from(vec![
        Span::styled(
            format!(" {} {text} ", state.icons.pr_state(d.state)),
            theme.fill(bg),
        ),
        Span::styled("  ", theme.body(bar)),
        Span::styled(d.title.clone(), theme.title(bar)),
        Span::styled(format!("  #{}", pr.number), theme.meta(bar)),
    ])
}
