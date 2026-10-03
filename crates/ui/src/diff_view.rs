//! Diff rendering, unified or split. Only the visible rows are laid out, so
//! cost per frame depends on the terminal height, not the size of the PR.

use ghtui_diff::{Content, DiffLine, LineKind, Span as TokenSpan, TextDiff, TokenKind};
use ghtui_git::files::{FileStatus, MODE_SUBMODULE, MODE_SYMLINK};
use ghtui_theme::{Bg, DiffBg, Fg, Syntax};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthChar;

use crate::diff_doc::{Doc, DocFile, Note, Pos, Row, Viewed};
use crate::{Ctx, PAD_X, chips, fill, text};

const PANE: Bg = Bg::Surface;
const HEADER: Bg = Bg::ContainerHigh;
const TAB_WIDTH: usize = 4;

pub fn syntax_role(kind: TokenKind) -> Syntax {
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
    pub expand: &'a str,
    pub viewed: &'a str,
}

pub struct DiffView<'a> {
    pub ctx: Ctx<'a>,
    pub doc: &'a Doc,
    pub cursor: Pos,
    /// First visible row.
    pub top: Pos,
    pub keys: Keys<'a>,
}

impl Widget for DiffView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, PANE);
        if self.doc.is_empty() || area.height == 0 {
            return;
        }
        let first = self.doc.to_global(self.top);
        for i in 0..area.height {
            let g = first + usize::from(i);
            if g >= self.doc.total_rows() {
                break;
            }
            let pos = self.doc.to_pos(g);
            let row_area = Rect {
                y: area.y + i,
                height: 1,
                ..area
            };
            self.render_row(pos, row_area, buf);
        }
        // Sticky header: the file you're in stays named at the top.
        let top = self.doc.clamp(self.top);
        if top.row > 0 {
            let header = Pos {
                file: top.file,
                row: 0,
            };
            self.render_row(header, Rect { height: 1, ..area }, buf);
        }
    }
}

impl DiffView<'_> {
    fn render_row(&self, pos: Pos, area: Rect, buf: &mut Buffer) {
        let Some(row) = self.doc.row(pos) else { return };
        let file = &self.doc.files[pos.file];
        let cursor = pos == self.doc.clamp(self.cursor);
        let theme = self.ctx.theme;
        let quiet_bg = if cursor { Bg::SelectedInactive } else { PANE };
        let indent = usize::from(PAD_X) + 2 * file.number_width() + 2;
        match row {
            Row::Header => self.header(file, cursor, area, buf),
            Row::Note(note) => self.note(file, note, quiet_bg, area, buf),
            Row::Gap { start, end } => {
                fill(buf, area, theme, quiet_bg);
                let n = end - start;
                Line::from(vec![
                    Span::styled(
                        format!(
                            "{}⋯ {n} unchanged line{} · ",
                            " ".repeat(indent),
                            if n == 1 { "" } else { "s" }
                        ),
                        theme.meta(quiet_bg),
                    ),
                    Span::styled(
                        self.keys.expand.to_owned(),
                        theme.accent(quiet_bg).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(" to expand", theme.meta(quiet_bg)),
                ])
                .render(area, buf);
            }
            Row::Hunk { seg } => {
                fill(buf, area, theme, quiet_bg);
                let header = file.headers().get(seg as usize).map_or("", String::as_str);
                Span::styled(
                    format!("{}{header}", " ".repeat(indent)),
                    theme.meta(quiet_bg),
                )
                .render(area, buf);
            }
            Row::Line(e) => self.unified(file, e, cursor, area, buf),
            Row::Split { left, right } => self.split(file, left, right, cursor, area, buf),
            Row::NoNewline => {
                fill(buf, area, theme, quiet_bg);
                Span::styled(
                    format!("{}\\ No newline at end of file", " ".repeat(indent + 2)),
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

    fn header(&self, file: &DocFile, cursor: bool, area: Rect, buf: &mut Buffer) {
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
        match file.viewed {
            Viewed::Viewed => chip(
                format!(
                    "{} Viewed",
                    ctx.icons.checks(ghtui_api::model::ChecksState::Passing)
                ),
                Bg::SuccessContainer,
            ),
            Viewed::Dismissed => chip("Changed since viewed".into(), Bg::TertiaryContainer),
            Viewed::Unviewed => {}
        }
        match meta.status {
            FileStatus::Added => chip("Added".into(), Bg::SuccessContainer),
            FileStatus::Deleted => chip("Deleted".into(), Bg::ErrorContainer),
            FileStatus::Renamed => chip(
                format!("Renamed {}%", meta.similarity.unwrap_or(0)),
                Bg::TertiaryContainer,
            ),
            FileStatus::Copied => chip(
                format!("Copied {}%", meta.similarity.unwrap_or(0)),
                Bg::TertiaryContainer,
            ),
            FileStatus::TypeChanged => chip("Type changed".into(), Bg::SecondaryContainer),
            FileStatus::Modified => {}
        }
        if meta.new_mode == MODE_SUBMODULE || meta.old_mode == MODE_SUBMODULE {
            chip("Submodule".into(), Bg::SecondaryContainer);
        } else if meta.is_symlink() {
            chip("Symlink".into(), Bg::SecondaryContainer);
        }
        if matches!(
            file.diff.as_deref().map(|d| &d.content),
            Some(Content::Binary { .. })
        ) {
            chip("Binary".into(), Bg::SecondaryContainer);
        }
        if file.generated {
            chip("Generated".into(), Bg::SecondaryContainer);
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
            .filter(|b| self.doc.reviewed.contains(&b.hash))
            .count();
        if reviewed > 0 {
            right.push(Span::styled(
                format!("{reviewed}/{} reviewed   ", blocks.len()),
                theme.style(Fg::Success, bg),
            ));
        }
        if file.diff.is_some() {
            let (adds, dels) = self.doc.file_counts(file);
            right.push(Span::styled(
                format!("+{adds}"),
                theme.style(Fg::DiffAddedSign, bg),
            ));
            right.push(Span::styled(" ", theme.body(bg)));
            right.push(Span::styled(
                format!("−{dels}"),
                theme.style(Fg::DiffRemovedSign, bg),
            ));
        }
        split_line(area, buf, left, right);
    }

    fn note(&self, file: &DocFile, note: Note, bg: Bg, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, bg);
        let content = file.diff.as_deref().map(|d| &d.content);
        let keys = self.keys;
        let (text, fg) = match (note, content) {
            (Note::Loading, _) => ("Loading diff…".to_owned(), Fg::OnSurfaceVariant),
            (Note::Collapsed, _) => (
                format!("Generated file, collapsed. Press {} to show it.", keys.show),
                Fg::OnSurfaceVariant,
            ),
            (Note::Viewed, _) => (
                format!(
                    "Viewed. Press {} to show it, or {} to mark it unviewed.",
                    keys.show, keys.viewed
                ),
                Fg::OnSurfaceVariant,
            ),
            (Note::Binary, Some(Content::Binary { old_size, new_size })) => (
                format!("Binary file: {} → {}", size(*old_size), size(*new_size)),
                Fg::OnSurfaceVariant,
            ),
            (Note::TooLarge, Some(Content::TooLarge { old_size, new_size })) => (
                format!(
                    "Too large to diff: {} → {}",
                    size(*old_size),
                    size(*new_size)
                ),
                Fg::OnSurfaceVariant,
            ),
            (Note::Submodule, Some(Content::Submodule { old, new })) => (
                format!(
                    "Submodule {} → {}",
                    short(old.as_deref()),
                    short(new.as_deref())
                ),
                Fg::OnSurfaceVariant,
            ),
            (Note::Error, Some(Content::Error(message))) => {
                (format!("Couldn't load this file: {message}"), Fg::Error)
            }
            (Note::NoChanges, _) => {
                let meta = &file.meta;
                let whitespace_only = file
                    .text()
                    .is_some_and(|t| t.has_changes(ghtui_diff::Whitespace::Exact));
                let text = if whitespace_only {
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
                };
                (text, Fg::OnSurfaceVariant)
            }
            _ => (String::new(), Fg::OnSurfaceVariant),
        };
        let inner = Rect {
            x: area.x + PAD_X,
            width: area.width.saturating_sub(2 * PAD_X),
            ..area
        };
        Span::styled(
            text::truncate(&text, usize::from(inner.width)),
            theme.style(fg, bg),
        )
        .render(inner, buf);
    }

    /// The reviewed mark column (1 cell).
    fn mark(&self, file: &DocFile, entry: Option<u32>, bg: Bg) -> Span<'static> {
        let reviewed = entry
            .and_then(|e| file.block_of(e))
            .is_some_and(|b| self.doc.reviewed.contains(&b.hash));
        if reviewed {
            Span::styled("✓", self.ctx.theme.style(Fg::Success, bg))
        } else {
            Span::styled(" ", self.ctx.theme.body(bg))
        }
    }

    fn unified(&self, file: &DocFile, e: u32, cursor: bool, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let Some(text) = file.text() else { return };
        let Some(line) = text.lines(self.doc.opts.whitespace).get(e as usize) else {
            return;
        };
        let diff_bg = diff_bg(line.kind);
        let bg = line_bg(diff_bg, cursor);
        fill(buf, area, theme, bg);
        let width = file.number_width();
        let mut spans = vec![
            Span::styled(" ", theme.body(bg)),
            self.mark(file, Some(e), bg),
            Span::styled(
                number(line.old, width),
                theme.style(self.gutter_fg(line.kind), bg),
            ),
            Span::styled(" ", theme.body(bg)),
            Span::styled(
                number(line.new, width),
                theme.style(self.gutter_fg(line.kind), bg),
            ),
            Span::styled("  ", theme.body(bg)),
        ];
        let used: usize = spans.iter().map(Span::width).sum();
        let room = usize::from(area.width).saturating_sub(used + 1);
        spans.extend(self.code(text, line, diff_bg, cursor, room));
        Line::from(spans).render(area, buf);
    }

    fn split(
        &self,
        file: &DocFile,
        left: Option<u32>,
        right: Option<u32>,
        cursor: bool,
        area: Rect,
        buf: &mut Buffer,
    ) {
        let theme = self.ctx.theme;
        let Some(text) = file.text() else { return };
        let lines = text.lines(self.doc.opts.whitespace);
        let width = file.number_width();
        let half = area.width.saturating_sub(1) / 2;
        let halves = [
            (
                left.and_then(|e| lines.get(e as usize)),
                Rect {
                    width: half,
                    ..area
                },
                true,
            ),
            (
                right.and_then(|e| lines.get(e as usize)),
                Rect {
                    x: area.x + half + 1,
                    width: area.width.saturating_sub(half + 1),
                    ..area
                },
                false,
            ),
        ];
        fill(buf, area, theme, line_bg(DiffBg::Context, cursor));
        for (line, half_area, is_left) in halves {
            let diff_bg = line.map_or(DiffBg::Context, |l| diff_bg(l.kind));
            let bg = line_bg(diff_bg, cursor);
            fill(buf, half_area, theme, bg);
            let mut spans = Vec::new();
            if is_left {
                spans.push(Span::styled(" ", theme.body(bg)));
                spans.push(self.mark(file, left.or(right), bg));
            }
            let Some(line) = line else {
                Line::from(spans).render(half_area, buf);
                continue;
            };
            let n = if is_left { line.old } else { line.new };
            spans.push(Span::styled(
                number(n, width),
                theme.style(self.gutter_fg(line.kind), bg),
            ));
            spans.push(Span::styled("  ", theme.body(bg)));
            let used: usize = spans.iter().map(Span::width).sum();
            let room = usize::from(half_area.width).saturating_sub(used + 1);
            spans.extend(self.code(text, line, diff_bg, cursor, room));
            Line::from(spans).render(half_area, buf);
        }
    }

    fn gutter_fg(&self, kind: LineKind) -> Fg {
        // When tints are indistinguishable (some 256-color palettes), carry
        // the change in the gutter instead.
        let marked = self.ctx.theme.diff_tints_collapse();
        match kind {
            LineKind::Added if marked => Fg::DiffAddedSign,
            LineKind::Removed if marked => Fg::DiffRemovedSign,
            _ => Fg::OnSurfaceVariant,
        }
    }

    /// Sign, code and line-ending marker for one alignment entry.
    fn code(
        &self,
        text: &TextDiff,
        line: &DiffLine,
        diff_bg: DiffBg,
        cursor: bool,
        room: usize,
    ) -> Vec<Span<'static>> {
        let theme = self.ctx.theme;
        let bg = line_bg(diff_bg, cursor);
        let (sign, sign_fg) = match line.kind {
            LineKind::Context => (" ", Fg::OnSurfaceVariant),
            LineKind::Added => ("+", Fg::DiffAddedSign),
            LineKind::Removed => ("-", Fg::DiffRemovedSign),
        };
        let mut spans = vec![
            Span::styled(sign, theme.style(sign_fg, bg).add_modifier(Modifier::BOLD)),
            Span::styled(" ", theme.body(bg)),
        ];
        let (source, n, tokens) = match line.kind {
            LineKind::Removed => {
                let n = line.old.unwrap_or(1);
                (&text.old, n, text.old_spans(n))
            }
            _ => {
                let n = line.new.unwrap_or(1);
                (&text.new, n, text.new_spans(n))
            }
        };
        let i = n as usize - 1;
        spans.extend(code_spans(
            self.ctx,
            source.line(i),
            tokens,
            diff_bg,
            cursor,
            room.saturating_sub(2),
        ));
        // Line endings only matter where they changed.
        if source.crlf[i] && line.kind != LineKind::Context {
            spans.push(Span::styled("␍", theme.meta(bg)));
        }
        spans
    }
}

fn diff_bg(kind: LineKind) -> DiffBg {
    match kind {
        LineKind::Context => DiffBg::Context,
        LineKind::Added => DiffBg::Added,
        LineKind::Removed => DiffBg::Removed,
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
) -> Vec<Span<'static>> {
    let theme = ctx.theme;
    let bg = line_bg(diff_bg, cursor);
    let style_for = |kind: Option<TokenKind>| -> Style {
        let role = kind.map_or(Syntax::Default, syntax_role);
        let style = theme.style(Fg::Syntax(role), bg);
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

    let mut out = Vec::new();
    let mut used = 0usize;
    for (s, e, kind) in segments {
        let Some(slice) = content.get(s..e) else {
            continue;
        };
        let mut piece = String::new();
        let mut truncated = false;
        for c in slice.chars() {
            let (text, w): (String, usize) = match c {
                '\t' => {
                    let w = TAB_WIDTH - used % TAB_WIDTH;
                    (" ".repeat(w), w)
                }
                c if c.is_control() => ("�".into(), 1),
                c => (c.to_string(), c.width().unwrap_or(0)),
            };
            if used + w > room {
                truncated = true;
                break;
            }
            piece.push_str(&text);
            used += w;
        }
        if !piece.is_empty() {
            out.push(Span::styled(piece, style_for(kind)));
        }
        if truncated {
            out.push(Span::styled("…", theme.meta(bg)));
            break;
        }
    }
    out
}

fn split_line(area: Rect, buf: &mut Buffer, left: Vec<Span<'_>>, right: Vec<Span<'_>>) {
    let inner = Rect {
        x: area.x + PAD_X.min(area.width / 2),
        width: area.width.saturating_sub(2 * PAD_X),
        ..area
    };
    let right_width = (right.iter().map(Span::width).sum::<usize>() as u16).min(inner.width);
    Line::from(left).render(
        Rect {
            width: inner.width.saturating_sub(right_width + 2),
            ..inner
        },
        buf,
    );
    Line::from(right).render(
        Rect {
            x: inner.right() - right_width,
            width: right_width,
            ..inner
        },
        buf,
    );
}

fn size(bytes: Option<usize>) -> String {
    match bytes {
        None => "none".into(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        Some(b) => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

fn short(oid: Option<&str>) -> &str {
    oid.map_or("none", |o| o.get(..7).unwrap_or(o))
}
