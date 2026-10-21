//! Diff rendering, unified or split. Only the visible rows are laid out, so
//! cost per frame depends on the terminal height, not the size of the PR.

use crate::diff::{Content, DiffLine, Span as TokenSpan, TextDiff, TokenKind, Whitespace};
use crate::git::files::{FileStatus, MODE_SUBMODULE, MODE_SYMLINK};
use crate::theme::{Bg, DiffBg, Fg, Syntax};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use std::ops::Range;

use crate::diff::anchor::{LinePos, Side};

use crate::ui::annotations::{Annotation, Place, ThreadRowKind};
use crate::ui::diff_doc::{Doc, DocFile, FoldReason, Note, Pos, Row, Viewed};
use crate::ui::page::{Tone, role_style};
use crate::ui::{
    Ctx, PAD_X, chips, cols, fill, fit_spans, inset, key_hints, render_split, text, time,
};

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
    /// The half of a split row the cursor asks for.
    pub half: Side,
    /// First visible row.
    pub top: Pos,
    pub keys: Keys<'a>,
    /// Visual line selection (anchor and cursor, either order).
    pub selection: Option<(Pos, Pos)>,
    /// Columns code is scrolled left by, when it doesn't wrap.
    pub shift: usize,
    /// The word cursor, by its column in the code of the cursor's line.
    pub word: Option<usize>,
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
        // A wrapped row draws more of its line's row.
        let line_pos = self.doc.line_start(pos);
        let part = pos.row - line_pos.row;
        let Some(row) = self.doc.row(line_pos) else {
            return;
        };
        let pos = line_pos;
        let Some(file) = self.doc.files.get(pos.file) else {
            return;
        };
        let cursor = pos == self.doc.clamp(self.cursor) || self.selected(pos);
        let theme = self.ctx.theme;
        let quiet_bg = if cursor { Bg::SelectedInactive } else { PANE };
        let indent = sign_column(file);
        let pad = Pad(indent);
        // `⋯ what · KEY to verb`, aligned with the code.
        let action = |buf: &mut Buffer, what: Span<'static>, key: &str, verb: &str| {
            fill(buf, area, theme, quiet_bg);
            let mut spans = vec![what];
            spans.extend(key_hints(theme, quiet_bg, &[(key, verb)]));
            Line::from(fit_spans(&spans, area.width)).render(area, buf);
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
            Row::Line(e) => self.line(pos, part, Some(e), None, cursor, area, buf),
            Row::Split { left, right } => {
                // Two halves with a column of context tint between them;
                // the cursor is on one half.
                fill(buf, area, theme, line_bg(DiffBg::Context, false));
                let on = self.doc.half_at(pos, self.half);
                let cursor = |side| cursor && on == Some(side);
                let (left_width, right_width) = split_widths(area.width);
                let right_area = Rect {
                    x: area.x + left_width + 1,
                    width: right_width,
                    ..area
                };
                let left_area = Rect {
                    width: left_width,
                    ..area
                };
                let halves = [
                    (left, Side::Left, left_area),
                    (right, Side::Right, right_area),
                ];
                for (entry, side, area) in halves {
                    self.line(pos, part, entry, Some(side), cursor(side), area, buf);
                }
            }
            Row::Thread(t) => self.thread_row(file, t, cursor, area, buf),
            Row::Fold { block, reason } => {
                let lines = file
                    .blocks()
                    .get(block as usize)
                    .map_or(0, |b| b.entries.len() as u64);
                let what = match reason {
                    FoldReason::Formatting => "Formatting-only change",
                    FoldReason::Seen => "Unchanged since your review",
                };
                let lines = format!("{lines} {}", crate::ui::text::noun(lines, "line"));
                let what = Span::styled(format!("{pad}⋯ {what}, {lines} · "), theme.meta(quiet_bg));
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
            Row::Wrapped { .. } => {}
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
        let mut chip = |text: String, chip_bg| chips::push(&mut left, ctx, text, chip_bg, bg);
        let passed = ctx.icons.checks(crate::api::model::ChecksState::Passing);
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
        // Folded by hand: nothing under it says so.
        if file.open == Some(false) {
            chip("Folded".into(), Bg::SecondaryContainer);
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
        // Asked to compare by syntax, a file that couldn't be says so.
        if let Some(why) = file.text().and_then(TextDiff::fallback)
            && self.doc.opts().structural
        {
            right.push(Span::styled(
                format!("by lines: {}   ", why.reason()),
                theme.meta(bg),
            ));
        }
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
        render_split(inset(area, PAD_X, 0), buf, &left, &right, 2);
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
            text::fit(&text, usize::from(inner.width)),
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

    /// The two marker cells before line numbers: reviewed, and threads (on
    /// `side`'s lines only, in a split half; reviewed on the left half).
    fn marks(&self, file: &DocFile, row: Row, side: Option<Side>, bg: Bg) -> [Span<'static>; 2] {
        let theme = self.ctx.theme;
        let reviewed = side != Some(Side::Right)
            && row
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
            .filter(|pos| side.is_none_or(|side| pos.side == side))
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

    /// Alignment entry `entry` of the row at `pos`: the row's marks (a
    /// split half's own), line numbers (both in unified view, `side`'s in
    /// split view) and code; screen line `part` of it when code wraps,
    /// under blanks in their place. Both halves of a split row start
    /// alike, so their code gets the same room.
    #[expect(clippy::too_many_arguments, reason = "a line and where it's drawn")]
    fn line(
        &self,
        pos: Pos,
        part: usize,
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
            // Nothing in it is new or gone: the sign says it's in the diff.
            Some((e, _)) if a.kept.contains(&e) => DiffBg::Context,
            Some((_, l)) => diff_bg(l),
            None => DiffBg::Context,
        };
        let bg = line_bg(diff_bg, cursor);
        fill(buf, area, theme, bg);
        let mut spans = Vec::new();
        spans.push(Span::styled(" ", theme.body(bg)));
        spans.extend(self.marks(file, row, side, bg));
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
        if part > 0 {
            for span in &mut spans {
                span.content = Pad(text::width(&span.content)).to_string().into();
            }
        }
        // Unified shows one line per entry; a split half shows its own side.
        if let Some(at) = side.map_or(Some(line.shown()), |side| line.on(side)) {
            let room = code_room(file, area.width, side.is_some());
            let (code, tokens) = (text.line(at), text.spans(at));
            // The word cursor, on the cursor's line (its half, if split).
            let on = pos == self.doc.clamp(self.cursor) && side == self.doc.half_at(pos, self.half);
            let word = (self.word.filter(|_| on))
                .and_then(|col| Some(text::word_at(&text::words(code), col)?.cols.clone()));
            let wraps = self.doc.opts().wrap_code.is_some();
            let cut = if wraps {
                Cut::Wrap(part)
            } else {
                Cut::Shift(self.shift)
            };
            spans.extend(code_spans(
                self.ctx,
                code,
                tokens,
                diff_bg,
                cursor,
                (room, cut),
                emphasis,
                word.as_ref(),
            ));
            let last = !wraps || code_rows(code, room) == part + 1;
            // Line endings only matter where they changed.
            if last && text.crlf(at) && line.is_change() {
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
        let card = thread_card(file, area);
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
                if ann.comments.iter().any(|c| c.new) {
                    chips::push(&mut spans, ctx, "New", Bg::TertiaryContainer, bg);
                }
                let n = ann.comments.len();
                let tail = format!("  {n} comment{}", if n == 1 { "" } else { "s" });
                let used = text::spans_width(&spans);
                let text_room = room.saturating_sub(used + text::width(&tail));
                spans.push(Span::styled(
                    text::truncate(&row.line.text(), text_room),
                    theme.body(bg),
                ));
                spans.push(Span::styled(tail, theme.meta(bg)));
            }
            ThreadRowKind::Head { comment, first } => {
                let Some(c) = ann.comments.get(*comment) else {
                    return;
                };
                spans.push(Span::styled(c.author.clone(), theme.title(bg)));
                if c.new {
                    spans.push(Span::styled(" ", theme.body(bg)));
                    spans.extend(chips::chip(
                        ctx,
                        "New",
                        theme.fill(Bg::TertiaryContainer),
                        bg,
                    ));
                }
                if !c.created_at.is_empty() {
                    spans.push(Span::styled(
                        format!(" · {}", time::ago_iso(&c.created_at, ctx.now)),
                        theme.meta(bg),
                    ));
                }
                if *first {
                    spans.push(Span::styled("  ", theme.body(bg)));
                    spans.extend(self.state_chips(ann, bg));
                    let place = match ann.place {
                        Place::Line {
                            at,
                            start: Some(start),
                        } if start != at.line => format!("lines {start}–{}", at.line),
                        Place::Outdated(Some(line)) => format!("line {line} of an earlier commit"),
                        _ => String::new(),
                    };
                    spans.push(Span::styled(place, theme.meta(bg)));
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
            // Rendered Markdown: code and a suggestion's lines on their
            // own tint.
            ThreadRowKind::Body => {
                let tint = |d| {
                    if cursor {
                        Bg::DiffSelected(d)
                    } else {
                        Bg::Diff(d)
                    }
                };
                let bg = match row.line.tone {
                    Tone::Plain => bg,
                    Tone::Code | Tone::Marked => tint(DiffBg::Context),
                    Tone::Added => tint(DiffBg::Added),
                    Tone::Removed => tint(DiffBg::Removed),
                };
                fill(buf, inner, theme, bg);
                let segs = row.line.segs.iter();
                spans.extend(
                    segs.map(|s| Span::styled(s.text().to_owned(), role_style(theme, &s.role, bg))),
                );
            }
            ThreadRowKind::Reactions { comment } => {
                let given = ann.comments.get(*comment).and_then(|c| c.editable.as_ref());
                for r in given.iter().flat_map(|e| &e.reactions) {
                    let fill = chips::reaction_fill(r.mine);
                    let text = format!("{} {}", r.content.emoji(), r.count);
                    chips::push(&mut spans, ctx, text, fill, bg);
                }
            }
            ThreadRowKind::Gap | ThreadRowKind::Error => {
                let style = match row.kind {
                    ThreadRowKind::Error => theme.error(bg),
                    _ => theme.meta(bg),
                };
                spans.push(Span::styled(text::truncate(&row.line.text(), room), style));
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
        Line::from(fit_spans(&spans, inner.width)).render(inner, buf);
    }

    fn state_chips(&self, ann: &Annotation, bg: Bg) -> Vec<Span<'static>> {
        let ctx = self.ctx;
        let mut out = Vec::new();
        let mut chip = |text: &str, chip_bg| chips::push(&mut out, ctx, text, chip_bg, bg);
        if ann.is_draft() {
            chip("Draft", Bg::TertiaryContainer);
            if ann.error.is_some() {
                chip("Rejected", Bg::ErrorContainer);
            }
        }
        if let Place::File { .. } = ann.place {
            chip("File", Bg::SecondaryContainer);
        }
        if ann.resolved {
            chip("Resolved", Bg::SuccessContainer);
        }
        match ann.place {
            Place::File { outdated: true } | Place::Outdated(_) => {
                chip("Outdated", Bg::SecondaryContainer);
            }
            Place::Moved(_) => chip("Outdated, moved", Bg::SecondaryContainer),
            Place::File { .. } | Place::Line { .. } => {}
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

/// Where a thread's card goes on a row of the diff `area`: from the sign
/// column to the pane's padding. Its text is [`PAD_X`] in.
fn thread_card(file: &DocFile, area: Rect) -> Rect {
    let indent = cols(sign_column(file)).min(area.width);
    Rect {
        x: area.x + indent,
        width: area.width.saturating_sub(indent + PAD_X),
        ..area
    }
}

/// The links in thread comments on screen, as the diff at `top` in `area`
/// draws them, top to bottom: where each starts, and where it leads.
pub fn link_spots(doc: &Doc, top: Pos, area: Rect) -> Vec<((u16, u16), String)> {
    let mut out = Vec::new();
    // The sticky header covers the first row once scrolled into a file.
    let covered = (doc.clamp(top).row > 0).then_some(area.top());
    let rows = doc.to_global(top)..doc.total_rows();
    for (g, y) in rows.zip(area.top()..area.bottom()) {
        let pos = doc.to_pos(g);
        let (Some(Row::Thread(t)), Some(file)) = (doc.row(pos), doc.files().get(pos.file)) else {
            continue;
        };
        let Some(row) = file.thread_row(t).filter(|_| covered != Some(y)) else {
            continue;
        };
        let mut x = thread_card(file, area).x.saturating_add(PAD_X);
        let mut seen = Vec::new();
        for seg in &row.line.segs {
            if let Some(link) = seg.link.filter(|l| !seen.contains(l)) {
                seen.push(link);
                if let Some(url) = row.line.links.get(link as usize) {
                    out.push(((x, y), url.clone()));
                }
            }
            x = x.saturating_add(cols(text::width(seg.text())));
        }
    }
    out
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

/// The widths of a split row's halves, in a row `width` wide: a column
/// between them.
pub fn split_widths(width: u16) -> (u16, u16) {
    let left = width.saturating_sub(1) / 2;
    (left, width.saturating_sub(left + 1))
}

/// Columns code gets in a row (or a split half) `width` wide: what the
/// marks, line numbers (one side's, if split), sign and a column for `…`
/// leave.
pub fn code_room(file: &DocFile, width: u16, split: bool) -> usize {
    let n = file.number_width();
    let numbers = if split { n } else { 2 * n + 1 };
    // A pad, two marks, the numbers, two spaces, `…`, the sign and a space.
    usize::from(width).saturating_sub(numbers + 8)
}

fn number(n: Option<u32>, width: usize) -> std::borrow::Cow<'static, str> {
    const BLANK: &str = "          ";
    match n {
        Some(n) => format!("{n:>width$}").into(),
        None => BLANK
            .get(..width)
            .map_or_else(|| " ".repeat(width).into(), Into::into),
    }
}

/// So many spaces, written where it's formatted: rows are drawn every
/// frame.
struct Pad(usize);

impl std::fmt::Display for Pad {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:1$}", "", self.0)
    }
}

/// The most screen lines a wrapped line of code takes; the rest of it is
/// cut, as minified code would fill screens.
const MAX_WRAPPED: usize = 100;

/// How code meets the edge of its room.
#[derive(Debug, Clone, Copy)]
enum Cut {
    /// Scrolled left by so many columns and cut at the edge.
    Shift(usize),
    /// Wrapped: this screen line of it.
    Wrap(usize),
}

/// A grapheme of code as drawn at column `col`: tabs expanded, hidden
/// characters as a marker (see `text::is_hidden`).
fn drawn(g: (&str, usize), col: usize) -> (&str, usize) {
    match text::untab(g, col) {
        (g, _) if g.contains(text::is_hidden) => ("�", 1),
        drawn => drawn,
    }
}

/// Screen lines `code` takes wrapped to `room` columns, as [`code_spans`]
/// wraps it.
pub fn code_rows(code: &str, room: usize) -> usize {
    let (mut col, mut used, mut rows) = (0usize, 0usize, 1usize);
    for g in text::graphemes(code) {
        let (_, w) = drawn(g, col);
        col += w;
        if used > 0 && used + w > room {
            if rows == MAX_WRAPPED {
                break;
            }
            rows += 1;
            used = 0;
        }
        used += w;
    }
    rows
}

/// Styled code, with tabs expanded and control characters made visible,
/// cut to `room` columns as `cut` says, the columns of `word` shown as the
/// word under the cursor.
#[expect(
    clippy::too_many_arguments,
    reason = "the code, its tokens and how it's drawn"
)]
fn code_spans(
    ctx: Ctx<'_>,
    content: &str,
    tokens: &[TokenSpan],
    diff_bg: DiffBg,
    cursor: bool,
    (room, cut): (usize, Cut),
    emphasis: &[(u32, u32)],
    word: Option<&Range<usize>>,
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
    let mut pieces: Vec<(usize, usize, Option<TokenKind>, bool)> = Vec::new();
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
            pieces.push((s, e, kind, emph));
        }
    }

    let mut out = Vec::new();
    // Columns of the line so far, and of them on this screen line; which
    // screen line it's at.
    let (mut col, mut used, mut at_line) = (0usize, 0usize, 0usize);
    let mut done = false;
    for (s, e, kind, emphasized) in pieces {
        let Some(slice) = content.get(s..e) else {
            continue;
        };
        let style = style_for(kind, emphasized);
        let styled = |piece: &mut String, lit: bool| {
            let style = if lit {
                style.add_modifier(Modifier::REVERSED)
            } else {
                style
            };
            Span::styled(std::mem::take(piece), style)
        };
        let (mut piece, mut lit) = (String::new(), false);
        let mut truncated = false;
        for g in text::graphemes(slice) {
            let (g, w) = drawn(g, col);
            col += w;
            match cut {
                // Scrolled off the left (a wide character half off, too).
                Cut::Shift(skip) if col - w < skip => continue,
                Cut::Shift(_) if used + w > room => {
                    truncated = true;
                    break;
                }
                Cut::Shift(_) => {}
                Cut::Wrap(n) => {
                    if used > 0 && used + w > room {
                        if at_line == n {
                            truncated = n + 1 == MAX_WRAPPED;
                            done = true;
                            break;
                        }
                        at_line += 1;
                        used = 0;
                    }
                    // On an earlier screen line.
                    if at_line < n {
                        used += w;
                        continue;
                    }
                }
            }
            let here = word.is_some_and(|word| word.contains(&(col - w)));
            if here != lit && !piece.is_empty() {
                out.push(styled(&mut piece, lit));
            }
            lit = here;
            piece.push_str(g);
            used += w;
        }
        if !piece.is_empty() {
            out.push(styled(&mut piece, lit));
        }
        if truncated {
            out.push(Span::styled("…", theme.meta(bg)));
        }
        if truncated || done {
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
    use crate::theme::{ColorDepth, DEFAULT_SEED, Mode, Theme};

    fn theme() -> Theme {
        Theme::new(DEFAULT_SEED, Mode::Dark, ColorDepth::TrueColor)
    }

    fn ctx(theme: &Theme) -> Ctx<'_> {
        Ctx {
            theme,
            icons: crate::ui::Icons::default(),
            now: 0,
        }
    }

    /// The text `code_spans` shows for an added `line`, cut `cut` to `room`.
    fn shown(line: &str, room: usize, cut: Cut) -> String {
        let theme = theme();
        let spans = code_spans(
            ctx(&theme),
            line,
            &[],
            DiffBg::Added,
            false,
            (room, cut),
            &[],
            None,
        );
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

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
        let line = "if a\u{202e} {\tb\u{200b}c";
        assert_eq!(shown(line, 80, Cut::Shift(0)), "if a� { b�c");
    }

    /// Wrapped, a long line's code goes on under it, all of it, with no
    /// `…`; tabs are as wide as they are unwrapped.
    #[test]
    fn wrapped_code_shows_all_of_the_line() {
        let line = format!("\t{}", "abcdefghij".repeat(5));
        let shown: Vec<String> = (0..code_rows(&line, 20))
            .map(|n| shown(&line, 20, Cut::Wrap(n)))
            .collect();
        assert_eq!(shown.len(), 3, "{shown:?}");
        assert!(shown.iter().all(|s| text::width(s) <= 20), "{shown:?}");
        assert_eq!(shown.concat(), line.replace('\t', "    "));
    }

    /// In split view with whitespace ignored, a re-indented line is one
    /// row, and each half shows its own side's text: the old indentation
    /// on the left, the new on the right.
    #[test]
    fn split_halves_show_their_own_side() {
        use crate::diff::{FileDiff, Whitespace};
        use crate::ui::diff_doc::ViewOptions;
        use std::collections::HashSet;
        use std::sync::Arc;

        let theme = theme();
        let mut doc = Doc::new(
            vec![crate::ui::diff_doc::tests::changed("a.rs")],
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
            ..ViewOptions::default()
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
            ctx: ctx(&theme),
            doc: &doc,
            cursor: Pos::default(),
            half: Side::Right,
            top: Pos::default(),
            keys,
            selection: None,
            shift: 0,
            word: None,
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
