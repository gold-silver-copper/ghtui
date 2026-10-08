//! Diff rendering, unified or split. Only the visible rows are laid out, so
//! cost per frame depends on the terminal height, not the size of the PR.

use ghtui_diff::{Content, DiffLine, Span as TokenSpan, TokenKind, Whitespace};
use ghtui_git::files::{FileStatus, MODE_SUBMODULE, MODE_SYMLINK};
use ghtui_theme::{Bg, DiffBg, Fg, Syntax};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use ghtui_diff::anchor::{LinePos, Side};

use crate::annotations::{Annotation, ThreadRowKind};
use crate::diff_doc::{Doc, DocFile, FoldReason, Note, Pos, Row, Viewed};
use crate::{Ctx, PAD_X, chips, cols, fill, inset, key_hints, render_split, text, time};

const PANE: Bg = Bg::Surface;
const HEADER: Bg = Bg::ContainerHigh;

pub(crate) fn syntax_role(kind: TokenKind) -> Syntax {
    match kind {
        TokenKind::Keyword => Syntax::Keyword,
        TokenKind::String => Syntax::String,
        TokenKind::Comment => Syntax::Comment,
        TokenKind::Function => Syntax::Function,
        TokenKind::Type => Syntax::Type,
        TokenKind::Number => Syntax::Number,
        TokenKind::Constant => Syntax::Constant,
        TokenKind::Operator => Syntax::Operator,
        TokenKind::Punctuation => Syntax::Punctuation,
        TokenKind::Attribute => Syntax::Attribute,
        TokenKind::Property => Syntax::Property,
    }
}

/// Keys named in hints, from the active keymap.
#[derive(Debug, Clone, Copy, Default)]
pub struct Keys<'a> {
    pub show: &'a str,
    pub jump: &'a str,
    pub expand: &'a str,
    pub viewed: &'a str,
    pub reply: &'a str,
    pub resolve: &'a str,
    pub delete: &'a str,
    pub file_comment: &'a str,
}

pub struct DiffView<'a> {
    pub ctx: Ctx<'a>,
    pub doc: &'a Doc,
    pub cursor: Pos,
    /// First visible row.
    pub top: Pos,
    pub keys: Keys<'a>,
    /// Visual line selection (anchor and cursor, either order).
    pub selection: Option<(Pos, Pos)>,
}

impl Widget for DiffView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, PANE);
        if self.doc.is_empty() || area.height == 0 {
            return;
        }
        let rows = self.doc.to_global(self.top)..self.doc.total_rows();
        for (g, y) in rows.zip(area.top()..area.bottom()) {
            let row_area = Rect {
                y,
                height: 1,
                ..area
            };
            self.render_row(self.doc.to_pos(g), row_area, buf);
        }
        // Sticky header: the file (and function) you're in stays named at
        // the top.
        let top = self.doc.clamp(self.top);
        if top.row > 0 {
            let cursor = self.doc.clamp(self.cursor) == Pos { row: 0, ..top };
            let scope = self.doc.scope_at(Pos {
                row: top.row + 1,
                ..top
            });
            self.header(top.file, cursor, &scope, Rect { height: 1, ..area }, buf);
        }
    }
}

impl DiffView<'_> {
    fn render_row(&self, pos: Pos, area: Rect, buf: &mut Buffer) {
        let Some(row) = self.doc.row(pos) else { return };
        let Some(file) = self.doc.files.get(pos.file) else {
            return;
        };
        let cursor = pos == self.doc.clamp(self.cursor) || self.selected(pos);
        let theme = self.ctx.theme;
        let quiet_bg = if cursor { Bg::SelectedInactive } else { PANE };
        let indent = sign_column(file);
        let pad = " ".repeat(indent);
        // `⋯ what · KEY to verb`, aligned with the code.
        let action = |buf: &mut Buffer, what: Span<'static>, key: &str, verb: &str| {
            fill(buf, area, theme, quiet_bg);
            let mut spans = vec![what];
            spans.extend(key_hints(theme, quiet_bg, &[(key, verb)]));
            Line::from(spans).render(area, buf);
        };
        match row {
            Row::Header => self.header(pos.file, cursor, &[], area, buf),
            Row::Note(note) => self.note(file, note, quiet_bg, area, buf),
            Row::Gap { start, end } => {
                let n = end - start;
                let what = match (self.doc.since_active, n == 1) {
                    (true, true) => "line without new changes",
                    (true, false) => "lines without new changes",
                    (false, true) => "unchanged line",
                    (false, false) => "unchanged lines",
                };
                let what = Span::styled(format!("{pad}⋯ {n} {what} · "), theme.meta(quiet_bg));
                action(buf, what, self.keys.expand, "to expand");
            }
            Row::Hunk { seg } => {
                fill(buf, area, theme, quiet_bg);
                let Some(header) = file.headers().get(seg as usize) else {
                    return;
                };
                let room =
                    usize::from(area.width).saturating_sub(indent + text::width(&header.range) + 1);
                let scope: Vec<&str> = header.scope.iter().map(String::as_str).collect();
                Line::from(vec![
                    Span::styled(format!("{pad}{} ", header.range), theme.meta(quiet_bg)),
                    Span::styled(scope_label(&scope, room), theme.body(quiet_bg)),
                ])
                .render(area, buf);
            }
            Row::Line(e) => self.line(pos, Some(e), None, cursor, area, buf),
            Row::Split { left, right } => {
                // Two halves with a column of context tint between them.
                fill(buf, area, theme, line_bg(DiffBg::Context, cursor));
                let half = area.width.saturating_sub(1) / 2;
                let right_area = Rect {
                    x: area.x + half + 1,
                    width: area.width.saturating_sub(half + 1),
                    ..area
                };
                let left_area = Rect {
                    width: half,
                    ..area
                };
                self.line(pos, left, Some(Side::Left), cursor, left_area, buf);
                self.line(pos, right, Some(Side::Right), cursor, right_area, buf);
            }
            Row::Thread(t) => self.thread_row(file, t, cursor, area, buf),
            Row::Fold { block, reason } => {
                let lines = file
                    .blocks()
                    .get(block as usize)
                    .map_or(0, |b| b.entries.len());
                let what = match reason {
                    FoldReason::Formatting => "Formatting-only change",
                    FoldReason::Seen => "Unchanged since your review",
                };
                let what = Span::styled(
                    format!("{pad}⋯ {what}, {lines} lines · "),
                    theme.meta(quiet_bg),
                );
                action(buf, what, self.keys.show, "to show");
            }
            Row::Moved { mv, from } => {
                let what = Span::styled(
                    format!("{pad}↳ {} · ", self.move_label(mv, from)),
                    theme.style(Fg::Tertiary, quiet_bg),
                );
                action(buf, what, self.keys.jump, "to jump");
            }
            Row::NoNewline => {
                fill(buf, area, theme, quiet_bg);
                Span::styled(
                    format!("{pad}  \\ No newline at end of file"),
                    theme.meta(quiet_bg).add_modifier(Modifier::ITALIC),
                )
                .render(area, buf);
            }
            Row::Spacer => {
                if cursor {
                    fill(buf, area, theme, Bg::SelectedInactive);
                }
            }
        }
    }

    /// A file's header row; `scope` names where you are in it (the sticky
    /// header).
    fn header(
        &self,
        file_index: usize,
        cursor: bool,
        scope: &[&str],
        area: Rect,
        buf: &mut Buffer,
    ) {
        let Some(file) = self.doc.files.get(file_index) else {
            return;
        };
        let ctx = self.ctx;
        let theme = ctx.theme;
        let bg = if cursor { Bg::SelectedHigh } else { HEADER };
        fill(buf, area, theme, bg);
        let meta = &file.meta;

        let mut left: Vec<Span<'static>> = Vec::new();
        let mut chip = |text: String, chip_bg: Bg| {
            left.extend(chips::chip(ctx, text, theme.fill(chip_bg), bg));
            left.push(Span::styled(" ", theme.body(bg)));
        };
        let passed = ctx.icons.checks(ghtui_api::model::ChecksState::Passing);
        match file.viewed {
            Viewed::Viewed => chip(format!("{passed} Viewed"), Bg::SuccessContainer),
            Viewed::Dismissed => chip("Changed since viewed".into(), Bg::TertiaryContainer),
            Viewed::Unviewed => {}
        }
        let similarity = meta.similarity.unwrap_or(0);
        match meta.status {
            FileStatus::Added => chip("Added".into(), Bg::SuccessContainer),
            FileStatus::Deleted => chip("Deleted".into(), Bg::ErrorContainer),
            FileStatus::Renamed => chip(format!("Renamed {similarity}%"), Bg::TertiaryContainer),
            FileStatus::Copied => chip(format!("Copied {similarity}%"), Bg::TertiaryContainer),
            FileStatus::TypeChanged => chip("Type changed".into(), Bg::SecondaryContainer),
            FileStatus::Modified => {}
        }
        if meta.new_mode == MODE_SUBMODULE || meta.old_mode == MODE_SUBMODULE {
            chip("Submodule".into(), Bg::SecondaryContainer);
        } else if meta.is_symlink() {
            chip("Symlink".into(), Bg::SecondaryContainer);
        }
        if let Some(Content::Binary { .. }) = file.diff.as_deref().map(|d| &d.content) {
            chip("Binary".into(), Bg::SecondaryContainer);
        }
        if file.generated {
            chip("Generated".into(), Bg::SecondaryContainer);
        }
        if self.doc.since_active && file.diff.is_some() && self.doc.has_new_changes(file_index) {
            chip("New since your review".into(), Bg::PrimaryContainer);
        }
        if file.full {
            chip("Full file".into(), Bg::SecondaryContainer);
        }
        if let Some(old) = meta.old_path.as_deref()
            && matches!(meta.status, FileStatus::Renamed | FileStatus::Copied)
        {
            left.push(Span::styled(format!("{old} → "), theme.meta(bg)));
        }
        left.push(Span::styled(meta.path().to_owned(), theme.title(bg)));
        if meta.mode_changed() && meta.old_mode != MODE_SYMLINK && meta.new_mode != MODE_SYMLINK {
            left.push(Span::styled(
                format!("  mode {:o} → {:o}", meta.old_mode, meta.new_mode),
                theme.meta(bg),
            ));
        }

        let mut right = Vec::new();
        let blocks = file.blocks();
        let reviewed = blocks
            .iter()
            .filter(|b| self.doc.inputs.reviewed.contains(&b.hash))
            .count();
        if reviewed > 0 {
            right.push(Span::styled(
                format!("{reviewed}/{} reviewed   ", blocks.len()),
                theme.style(Fg::Success, bg),
            ));
        }
        if file.diff.is_some() {
            let (adds, dels) = self.doc.file_counts(file);
            right.extend([
                Span::styled(format!("+{adds}"), theme.style(Fg::DiffAddedSign, bg)),
                Span::styled(" ", theme.body(bg)),
                Span::styled(format!("−{dels}"), theme.style(Fg::DiffRemovedSign, bg)),
            ]);
        }
        if !scope.is_empty() {
            // What's left between the file name and the counts.
            let used = text::spans_width(left.iter().chain(&right));
            let room = usize::from(area.width).saturating_sub(used + 2 * usize::from(PAD_X) + 5);
            left.push(Span::styled(
                format!(" › {}", scope_label(scope, room)),
                theme.meta(bg),
            ));
        }
        render_split(inset(area, PAD_X, 0), buf, left, right, 2);
    }

    fn note(&self, file: &DocFile, note: Note, bg: Bg, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, bg);
        let content = file.diff.as_deref().map(|d| &d.content);
        let keys = self.keys;
        let text = match (note, content) {
            (Note::Loading, _) => "Loading diff…".to_owned(),
            (Note::Collapsed, _) => {
                format!("Generated file, collapsed. Press {} to show it.", keys.show)
            }
            (Note::Viewed, _) => format!(
                "Viewed. Press {} to show it, or {} to mark it unviewed.",
                keys.show, keys.viewed
            ),
            (Note::Binary, Some(Content::Binary { old_size, new_size })) => {
                format!("Binary file: {} → {}", size(*old_size), size(*new_size))
            }
            (Note::TooLarge, Some(Content::TooLarge { old_size, new_size })) => format!(
                "Too large to diff: {} → {}",
                size(*old_size),
                size(*new_size)
            ),
            (Note::Submodule, Some(Content::Submodule { old, new })) => format!(
                "Submodule {} → {}",
                short(old.as_deref()),
                short(new.as_deref())
            ),
            (Note::Error, Some(Content::Error(message))) => {
                format!("Couldn't load this file: {message}")
            }
            (Note::NothingNew, _) => "Nothing new since your last review.".to_owned(),
            (Note::NoChanges, _) => {
                let meta = &file.meta;
                let whitespace_only = file
                    .text()
                    .is_some_and(|t| !t.alignment(Whitespace::Exact).blocks.is_empty());
                if whitespace_only {
                    "Only whitespace changed (whitespace is being ignored).".to_owned()
                } else if meta.status == FileStatus::Renamed {
                    "Renamed without content changes.".to_owned()
                } else if meta.mode_changed() {
                    format!(
                        "Mode changed {:o} → {:o}, contents unchanged.",
                        meta.old_mode, meta.new_mode
                    )
                } else {
                    "No content changes.".to_owned()
                }
            }
            _ => String::new(),
        };
        let fg = if note == Note::Error {
            Fg::Error
        } else {
            Fg::OnSurfaceVariant
        };
        let inner = inset(area, PAD_X, 0);
        Span::styled(
            text::truncate(&text, usize::from(inner.width)),
            theme.style(fg, bg),
        )
        .render(inner, buf);
    }

    /// "moved from path:line" / "moved to path:line".
    fn move_label(&self, mv: u32, from: bool) -> String {
        let Some(m) = self.doc.moves.get(mv as usize) else {
            return String::new();
        };
        let (file, entries) = if from { &m.to } else { &m.from };
        let Some(f) = self.doc.files.get(*file) else {
            return String::new();
        };
        let line = f.text().and_then(|t| {
            let l = (t.alignment(Whitespace::Exact).lines).get(entries.start as usize)?;
            Some(if from { l.right() } else { l.left() }?.line)
        });
        let way = if from { "to" } else { "from" };
        match line {
            Some(n) => format!("moved {way} {}:{n}", f.meta.path()),
            None => format!("moved {way} {}", f.meta.path()),
        }
    }

    fn selected(&self, pos: Pos) -> bool {
        let Some((a, b)) = self.selection else {
            return false;
        };
        let (a, b) = (self.doc.to_global(a), self.doc.to_global(b));
        let g = self.doc.to_global(pos);
        (a.min(b)..=a.max(b)).contains(&g)
    }

    /// The two marker cells before line numbers: reviewed, and threads.
    fn marks(&self, file: &DocFile, row: Row, bg: Bg) -> [Span<'static>; 2] {
        let theme = self.ctx.theme;
        let reviewed = row
            .entries()
            .filter_map(|e| file.block_of(e))
            .any(|b| self.doc.inputs.reviewed.contains(&b.hash));
        let reviewed = if reviewed {
            Span::styled("✓", theme.style(Fg::Success, bg))
        } else {
            Span::styled(" ", theme.body(bg))
        };
        let anns = file
            .shows(row)
            .flat_map(|pos| file.annotations_at(pos))
            .filter_map(|i| self.doc.annotations().get(*i as usize));
        let (mut any, mut draft, mut open) = (false, false, false);
        for a in anns {
            any = true;
            draft |= a.is_draft();
            open |= !a.resolved;
        }
        let thread = if draft {
            Span::styled("✎", theme.style(Fg::Tertiary, bg))
        } else if open {
            Span::styled("◆", theme.style(Fg::Primary, bg))
        } else if any {
            Span::styled("◇", theme.meta(bg))
        } else {
            Span::styled(" ", theme.body(bg))
        };
        [reviewed, thread]
    }

    /// Line numbers GitHub won't accept comments on are dimmed.
    fn number_fg(&self, file_index: usize, pos: Option<LinePos>, line: &DiffLine) -> Fg {
        let commentable = match (pos, self.doc.commentable(file_index)) {
            (Some(pos), Some(c)) => c.is_commentable(pos),
            _ => true,
        };
        if commentable {
            self.gutter_fg(line)
        } else {
            Fg::Disabled
        }
    }

    /// Alignment entry `entry` of the row at `pos`: the row's marks (except
    /// on a split row's right half), line numbers (both in unified view,
    /// `side`'s in split view) and code.
    fn line(
        &self,
        pos: Pos,
        entry: Option<u32>,
        side: Option<Side>,
        cursor: bool,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let theme = self.ctx.theme;
        let (Some(file), Some(row)) = (self.doc.files.get(pos.file), self.doc.row(pos)) else {
            return;
        };
        let (Some(text), Some(a)) = (file.text(), file.alignment()) else {
            return;
        };
        let line = entry.and_then(|e| Some((e, a.lines.get(e as usize)?)));
        let moved = line.is_some_and(|(e, _)| self.doc.move_at_entry(pos.file, e).is_some());
        let diff_bg = match line {
            Some(_) if moved => DiffBg::Moved,
            Some((_, l)) => diff_bg(l),
            None => DiffBg::Context,
        };
        let bg = line_bg(diff_bg, cursor);
        fill(buf, area, theme, bg);
        let mut spans = Vec::new();
        if side != Some(Side::Right) {
            spans.push(Span::styled(" ", theme.body(bg)));
            spans.extend(self.marks(file, row, bg));
        }
        let Some((e, line)) = line else {
            Line::from(spans).render(area, buf);
            return;
        };
        let numbers = [Side::Left, Side::Right]
            .into_iter()
            .filter(|s| side.is_none_or(|side| side == *s));
        for (i, at) in numbers.map(|s| line.on(s)).enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ", theme.body(bg)));
            }
            spans.push(Span::styled(
                number(at.map(|p| p.line), file.number_width()),
                theme.style(self.number_fg(pos.file, at, line), bg),
            ));
        }
        spans.push(Span::styled("  ", theme.body(bg)));
        let used = text::spans_width(&spans);
        let room = usize::from(area.width).saturating_sub(used + 1);
        // No emphasis on moved lines: the whole block is the change.
        let emphasis = match a.intraline.get(&e) {
            Some(ranges) if !moved => ranges.as_slice(),
            _ => &[],
        };
        let (sign, sign_fg) = match line {
            DiffLine::Context { .. } => (" ", Fg::OnSurfaceVariant),
            DiffLine::Added(_) => ("+", Fg::DiffAddedSign),
            DiffLine::Removed(_) => ("-", Fg::DiffRemovedSign),
        };
        spans.push(Span::styled(
            sign,
            theme.style(sign_fg, bg).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(" ", theme.body(bg)));
        // Unified shows one line per entry; a split half shows its own side.
        if let Some(at) = side.map_or(Some(line.shown()), |side| line.on(side)) {
            let room = room.saturating_sub(2);
            let (code, tokens) = (text.line(at), text.spans(at));
            spans.extend(code_spans(
                self.ctx, code, tokens, diff_bg, cursor, room, emphasis,
            ));
            // Line endings only matter where they changed.
            if text.crlf(at) && line.is_change() {
                spans.push(Span::styled("␍", theme.meta(bg)));
            }
        }
        Line::from(spans).render(area, buf);
    }

    fn thread_row(&self, file: &DocFile, t: u32, cursor: bool, area: Rect, buf: &mut Buffer) {
        let ctx = self.ctx;
        let theme = ctx.theme;
        let Some(row) = file.thread_row(t) else {
            return;
        };
        let Some(ann) = self.doc.annotations().get(row.ann as usize) else {
            return;
        };
        fill(buf, area, theme, PANE);
        let indent = cols(sign_column(file)).min(area.width);
        let card = Rect {
            x: area.x + indent,
            width: area.width.saturating_sub(indent + PAD_X),
            ..area
        };
        let bg = if cursor {
            Bg::Selected
        } else {
            Bg::ContainerLow
        };
        fill(buf, card, theme, bg);
        let inner = inset(card, PAD_X, 0);
        let room = usize::from(inner.width);
        let keys = self.keys;
        let mut spans: Vec<Span<'static>> = Vec::new();
        match &row.kind {
            ThreadRowKind::Summary => {
                let (mark, fg) = if ann.resolved {
                    ("◇", Fg::OnSurfaceVariant)
                } else {
                    ("◆", Fg::Primary)
                };
                spans.push(Span::styled(format!("{mark} "), theme.style(fg, bg)));
                spans.extend(self.state_chips(ann, bg));
                let n = ann.comments.len();
                let tail = format!("  {n} comment{}", if n == 1 { "" } else { "s" });
                let used = text::spans_width(&spans);
                let text_room = room.saturating_sub(used + text::width(&tail));
                spans.push(Span::styled(
                    text::truncate(&row.text, text_room),
                    theme.body(bg),
                ));
                spans.push(Span::styled(tail, theme.meta(bg)));
            }
            ThreadRowKind::Head { comment, first } => {
                let Some(c) = ann.comments.get(*comment) else {
                    return;
                };
                spans.push(Span::styled(c.author.clone(), theme.title(bg)));
                if !c.created_at.is_empty() {
                    spans.push(Span::styled(
                        format!(" · {}", time::ago_iso(&c.created_at, ctx.now)),
                        theme.meta(bg),
                    ));
                }
                if *first {
                    spans.push(Span::styled("  ", theme.body(bg)));
                    spans.extend(self.state_chips(ann, bg));
                    if let (Some(start), Some(end)) = (ann.start_line, ann.line)
                        && start != end
                    {
                        spans.push(Span::styled(format!("lines {start}–{end}"), theme.meta(bg)));
                    }
                    if ann.outdated
                        && let Some(line) = ann.original_line
                    {
                        spans.push(Span::styled(
                            format!("line {line} of an earlier commit"),
                            theme.meta(bg),
                        ));
                    }
                }
                if c.pending {
                    spans.push(Span::styled("  ", theme.body(bg)));
                    spans.extend(chips::chip(
                        ctx,
                        "Pending",
                        theme.fill(Bg::SecondaryContainer),
                        bg,
                    ));
                }
            }
            ThreadRowKind::Body | ThreadRowKind::Gap | ThreadRowKind::Error => {
                let style = match row.kind {
                    ThreadRowKind::Error => theme.error(bg),
                    ThreadRowKind::Gap => theme.meta(bg),
                    _ => theme.body(bg),
                };
                spans.push(Span::styled(text::truncate(&row.text, room), style));
            }
            ThreadRowKind::Footer => {
                let mut hints = Vec::new();
                if ann.is_draft() {
                    if ann.error.is_some() {
                        hints.push((keys.file_comment, "post as a file comment"));
                    }
                    hints.extend([(keys.show, "edit"), (keys.delete, "delete")]);
                } else {
                    hints.push((keys.show, "collapse"));
                    if ann.can_reply {
                        hints.push((keys.reply, "reply"));
                    }
                    if ann.resolved && ann.can_unresolve {
                        hints.push((keys.resolve, "unresolve"));
                    } else if !ann.resolved && ann.can_resolve {
                        hints.push((keys.resolve, "resolve"));
                    }
                }
                spans = key_hints(theme, bg, &hints);
            }
        }
        Line::from(spans).render(inner, buf);
    }

    fn state_chips(&self, ann: &Annotation, bg: Bg) -> Vec<Span<'static>> {
        let ctx = self.ctx;
        let theme = ctx.theme;
        let mut out = Vec::new();
        let mut chip = |text: &str, chip_bg: Bg| {
            out.extend(chips::chip(ctx, text.to_owned(), theme.fill(chip_bg), bg));
            out.push(Span::styled(" ", theme.body(bg)));
        };
        if ann.is_draft() {
            chip("Draft", Bg::TertiaryContainer);
            if ann.error.is_some() {
                chip("Rejected", Bg::ErrorContainer);
            }
        }
        if ann.file_level {
            chip("File", Bg::SecondaryContainer);
        }
        if ann.resolved {
            chip("Resolved", Bg::SuccessContainer);
        }
        if ann.outdated {
            chip("Outdated", Bg::SecondaryContainer);
        } else if ann.moved {
            chip("Outdated, moved", Bg::SecondaryContainer);
        }
        out
    }

    fn gutter_fg(&self, line: &DiffLine) -> Fg {
        // When tints are indistinguishable (some 256-color palettes), carry
        // the change in the gutter instead.
        let marked = self.ctx.theme.diff_tints_collapse();
        match line {
            DiffLine::Added(_) if marked => Fg::DiffAddedSign,
            DiffLine::Removed(_) if marked => Fg::DiffRemovedSign,
            _ => Fg::OnSurfaceVariant,
        }
    }
}

/// `impl Doc › fn offset`, in at most `room` columns: outer scopes go
/// first, so the innermost (where you are) stays named.
fn scope_label(scope: &[&str], room: usize) -> String {
    (0..scope.len())
        .map(|skip| {
            let shown = scope.get(skip..).unwrap_or_default().join(" › ");
            if skip == 0 {
                shown
            } else {
                format!("… › {shown}")
            }
        })
        .find(|label| text::width(label) <= room)
        .unwrap_or_else(|| text::truncate(scope.last().copied().unwrap_or_default(), room))
}

/// Column of the +/- sign: hunk headers and gap text align to it, thread
/// cards start there.
fn sign_column(file: &DocFile) -> usize {
    // pad, reviewed mark, thread mark, old number, space, new number, 2 spaces
    3 + 2 * file.number_width() + 3
}

fn diff_bg(line: &DiffLine) -> DiffBg {
    match line {
        DiffLine::Context { .. } => DiffBg::Context,
        DiffLine::Added(_) => DiffBg::Added,
        DiffLine::Removed(_) => DiffBg::Removed,
    }
}

fn line_bg(diff_bg: DiffBg, cursor: bool) -> Bg {
    if cursor {
        Bg::DiffSelected(diff_bg)
    } else {
        Bg::Diff(diff_bg)
    }
}

fn number(n: Option<u32>, width: usize) -> String {
    n.map_or_else(|| " ".repeat(width), |n| format!("{n:>width$}"))
}

/// Styled code, with tabs expanded and control characters made visible,
/// cut to `room` columns.
fn code_spans(
    ctx: Ctx<'_>,
    content: &str,
    tokens: &[TokenSpan],
    diff_bg: DiffBg,
    cursor: bool,
    room: usize,
    emphasis: &[(u32, u32)],
) -> Vec<Span<'static>> {
    let theme = ctx.theme;
    let bg = line_bg(diff_bg, cursor);
    // Changed tokens within a changed line get the stronger tint.
    let strong = match diff_bg {
        DiffBg::Added => line_bg(DiffBg::AddedToken, cursor),
        DiffBg::Removed => line_bg(DiffBg::RemovedToken, cursor),
        _ => bg,
    };
    let style_for = |kind: Option<TokenKind>, emphasized: bool| -> Style {
        let role = kind.map_or(Syntax::Default, syntax_role);
        let style = theme.style(Fg::Syntax(role), if emphasized { strong } else { bg });
        if role == Syntax::Comment {
            style.add_modifier(Modifier::ITALIC)
        } else {
            style
        }
    };

    // Segments of (byte range, token kind) covering the whole line.
    let mut segments: Vec<(usize, usize, Option<TokenKind>)> = Vec::new();
    let mut at = 0usize;
    for t in tokens {
        let (s, e) = (t.start as usize, (t.end as usize).min(content.len()));
        if s < at || s >= e {
            continue;
        }
        if s > at {
            segments.push((at, s, None));
        }
        segments.push((s, e, Some(t.kind)));
        at = e;
    }
    if at < content.len() {
        segments.push((at, content.len(), None));
    }
    // Cut segments at emphasis boundaries.
    let mut cut: Vec<(usize, usize, Option<TokenKind>, bool)> = Vec::new();
    let mut bounds: Vec<usize> = Vec::new();
    for (s, e, kind) in segments {
        bounds.clear();
        bounds.extend([s, e]);
        for (a, b) in emphasis {
            for x in [*a as usize, *b as usize] {
                if x > s && x < e {
                    bounds.push(x);
                }
            }
        }
        bounds.sort_unstable();
        bounds.dedup();
        for &[s, e] in bounds.array_windows() {
            let emph = emphasis
                .iter()
                .any(|(a, b)| (*a as usize) <= s && e <= (*b as usize));
            cut.push((s, e, kind, emph));
        }
    }

    let mut out = Vec::new();
    let mut used = 0usize;
    for (s, e, kind, emphasized) in cut {
        let Some(slice) = content.get(s..e) else {
            continue;
        };
        let mut piece = String::new();
        let mut truncated = false;
        for g in text::graphemes(slice) {
            let (g, w) = match text::untab(g, used) {
                // Shown, never acted on: see `text::is_hidden`.
                (g, _) if g.contains(text::is_hidden) => ("�", 1),
                drawn => drawn,
            };
            if used + w > room {
                truncated = true;
                break;
            }
            piece.push_str(g);
            used += w;
        }
        if !piece.is_empty() {
            out.push(Span::styled(piece, style_for(kind, emphasized)));
        }
        if truncated {
            out.push(Span::styled("…", theme.meta(bg)));
            break;
        }
    }
    out
}

fn size(bytes: Option<usize>) -> String {
    bytes.map_or_else(|| "none".into(), |b| text::size(b as u64))
}

fn short(oid: Option<&str>) -> &str {
    oid.map_or("none", text::short_sha)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghtui_theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};

    /// Short of room, outer scopes go first; the innermost stays.
    #[test]
    fn scope_labels_keep_the_innermost() {
        let scope = ["impl Doc", "fn offset"];
        assert_eq!(scope_label(&scope, 40), "impl Doc › fn offset");
        assert_eq!(scope_label(&scope, 15), "… › fn offset");
        assert_eq!(scope_label(&scope, 6), "fn of…");
        assert_eq!(scope_label(&[], 10), "");
    }

    /// A bidi override in code ("Trojan Source") is shown, not obeyed or
    /// silently dropped; tabs expand without per-character allocation.
    #[test]
    fn hidden_characters_in_code_are_shown() {
        let theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor);
        let ctx = Ctx {
            theme: &theme,
            icons: crate::Icons::default(),
            now: 0,
        };
        let line = "if a\u{202e} {\tb\u{200b}c";
        let spans = code_spans(ctx, line, &[], DiffBg::Added, false, 80, &[]);
        let shown: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(shown, "if a� { b�c");
    }

    /// In split view with whitespace ignored, a re-indented line is one
    /// row, and each half shows its own side's text: the old indentation
    /// on the left, the new on the right.
    #[test]
    fn split_halves_show_their_own_side() {
        use crate::diff_doc::ViewOptions;
        use ghtui_diff::{FileDiff, Whitespace};
        use std::collections::HashSet;
        use std::sync::Arc;

        let theme = Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor);
        let ctx = Ctx {
            theme: &theme,
            icons: crate::Icons::default(),
            now: 0,
        };
        let mut doc = Doc::new(
            vec![crate::diff_doc::tests::changed("a.rs")],
            &HashSet::new(),
            Default::default(),
        );
        let (old, new) = ("a\n  b\nc\n", "a\n      b\nc\nd\n");
        doc.set_diff(
            0,
            Arc::new(FileDiff::compute(
                "a.rs",
                Some(old.as_bytes()),
                Some(new.as_bytes()),
            )),
        );
        doc.set_options(ViewOptions {
            split: true,
            whitespace: Whitespace::Ignore,
            wrap: 0,
        });
        let keys = Keys {
            show: "s",
            jump: "M",
            expand: "e",
            viewed: "v",
            reply: "c",
            resolve: "R",
            delete: "d",
            file_comment: "C",
        };
        let area = Rect::new(0, 0, 80, 12);
        let mut buf = Buffer::empty(area);
        DiffView {
            ctx,
            doc: &doc,
            cursor: Pos::default(),
            top: Pos::default(),
            keys,
            selection: None,
        }
        .render(area, &mut buf);
        let row = |y: u16, xs: std::ops::Range<u16>| -> String {
            xs.map(|x| buf[(x, y)].symbol()).collect()
        };
        let y = (0..area.height)
            .find(|y| row(*y, 0..80).contains('b'))
            .expect("the re-indented line is drawn");
        let (left, right) = (row(y, 0..40), row(y, 40..80));
        // Both halves lay out alike (number, sign, code), so the old line's
        // two spaces of indent leave it four columns left of the new one.
        let indent = |half: &str| {
            half.find('b')
                .unwrap_or(0)
                .saturating_sub(half.find('2').unwrap_or(0))
        };
        assert_eq!(
            indent(&left) + 4,
            indent(&right),
            "left half: {left:?}, right half: {right:?}"
        );
    }
}
