//! Unified diff rendering. Only the visible rows are laid out, so cost per
//! frame depends on the terminal height, not the size of the PR.

use ghtui_diff::{Content, LineKind, Span as TokenSpan, TokenKind};
use ghtui_git::files::{FileStatus, MODE_SUBMODULE, MODE_SYMLINK};
use ghtui_theme::{Bg, DiffBg, Fg, Syntax};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use unicode_width::UnicodeWidthChar;

use crate::diff_doc::{Doc, DocFile, Note, Pos, Row};
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

pub struct DiffView<'a> {
    pub ctx: Ctx<'a>,
    pub doc: &'a Doc,
    pub cursor: Pos,
    /// First visible row.
    pub top: Pos,
    /// Shown on collapsed files, e.g. "<Enter>".
    pub expand_key: &'a str,
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
        match row {
            Row::Header => self.header(file, cursor, area, buf),
            Row::Note(note) => self.note(file, note, cursor, area, buf),
            Row::Hunk(h) => self.hunk_header(file, h, cursor, area, buf),
            Row::Line { hunk, line } => self.line(file, hunk, line, cursor, area, buf),
            Row::NoNewline => {
                let bg = if cursor { Bg::SelectedInactive } else { PANE };
                fill(buf, area, self.ctx.theme, bg);
                let indent = PAD_X as usize + 2 * file.number_width() + 4;
                Span::styled(
                    format!("{}\\ No newline at end of file", " ".repeat(indent)),
                    self.ctx.theme.meta(bg).add_modifier(Modifier::ITALIC),
                )
                .render(area, buf);
            }
            Row::Spacer => {
                if cursor {
                    fill(buf, area, self.ctx.theme, Bg::SelectedInactive);
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

        let right = match file.diff.as_deref() {
            Some(diff) => vec![
                Span::styled(
                    format!("+{}", diff.additions),
                    theme.style(Fg::DiffAddedSign, bg),
                ),
                Span::styled(" ", theme.body(bg)),
                Span::styled(
                    format!("−{}", diff.deletions),
                    theme.style(Fg::DiffRemovedSign, bg),
                ),
            ],
            None => Vec::new(),
        };
        split_line(area, buf, left, right);
    }

    fn note(&self, file: &DocFile, note: Note, cursor: bool, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let bg = if cursor { Bg::SelectedInactive } else { PANE };
        fill(buf, area, theme, bg);
        let content = file.diff.as_deref().map(|d| &d.content);
        let (text, fg) = match (note, content) {
            (Note::Loading, _) => ("Loading diff…".to_owned(), Fg::OnSurfaceVariant),
            (Note::Collapsed, _) => (
                format!(
                    "Generated file, collapsed. Press {} to show it.",
                    self.expand_key
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
                let text = if meta.status == FileStatus::Renamed {
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

    fn hunk_header(&self, file: &DocFile, h: u32, cursor: bool, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        let bg = if cursor { Bg::SelectedInactive } else { PANE };
        fill(buf, area, theme, bg);
        let Some(Content::Text(text)) = file.diff.as_deref().map(|d| &d.content) else {
            return;
        };
        let Some(hunk) = text.hunks.get(h as usize) else {
            return;
        };
        let indent = PAD_X as usize + 2 * file.number_width() + 2;
        Span::styled(
            format!("{}{}", " ".repeat(indent), hunk.header()),
            theme.meta(bg),
        )
        .render(area, buf);
    }

    fn line(&self, file: &DocFile, h: u32, l: u32, cursor: bool, area: Rect, buf: &mut Buffer) {
        let ctx = self.ctx;
        let theme = ctx.theme;
        let Some(Content::Text(text)) = file.diff.as_deref().map(|d| &d.content) else {
            return;
        };
        let Some(line) = text
            .hunks
            .get(h as usize)
            .and_then(|hunk| hunk.lines.get(l as usize))
        else {
            return;
        };
        let diff_bg = match line.kind {
            LineKind::Context => DiffBg::Context,
            LineKind::Added => DiffBg::Added,
            LineKind::Removed => DiffBg::Removed,
        };
        let bg = if cursor {
            Bg::DiffSelected(diff_bg)
        } else {
            Bg::Diff(diff_bg)
        };
        fill(buf, area, theme, bg);

        let width = file.number_width();
        let number =
            |n: Option<u32>| n.map_or_else(|| " ".repeat(width), |n| format!("{n:>width$}"));
        // When tints are indistinguishable (some 256-color palettes), carry
        // the change in the gutter instead.
        let marked = theme.diff_tints_collapse() && line.kind != LineKind::Context;
        let gutter_fg = match line.kind {
            LineKind::Added if marked => Fg::DiffAddedSign,
            LineKind::Removed if marked => Fg::DiffRemovedSign,
            _ => Fg::OnSurfaceVariant,
        };
        let (sign, sign_fg) = match line.kind {
            LineKind::Context => (" ", Fg::OnSurfaceVariant),
            LineKind::Added => ("+", Fg::DiffAddedSign),
            LineKind::Removed => ("-", Fg::DiffRemovedSign),
        };
        let mut spans = vec![
            Span::styled(" ".repeat(usize::from(PAD_X)), theme.body(bg)),
            Span::styled(number(line.old), theme.style(gutter_fg, bg)),
            Span::styled(" ", theme.body(bg)),
            Span::styled(number(line.new), theme.style(gutter_fg, bg)),
            Span::styled("  ", theme.body(bg)),
            Span::styled(sign, theme.style(sign_fg, bg).add_modifier(Modifier::BOLD)),
            Span::styled(" ", theme.body(bg)),
        ];
        let used: usize = spans.iter().map(Span::width).sum();
        let room = usize::from(area.width).saturating_sub(used + 1);

        let (source, number, tokens, crlf) = match line.kind {
            LineKind::Removed => {
                let n = line.old.unwrap_or(1);
                (
                    &text.old,
                    n,
                    text.old_spans(n),
                    text.old.crlf[n as usize - 1],
                )
            }
            _ => {
                let n = line.new.unwrap_or(1);
                (
                    &text.new,
                    n,
                    text.new_spans(n),
                    text.new.crlf[n as usize - 1],
                )
            }
        };
        let content = source.line(number as usize - 1);
        spans.extend(code_spans(ctx, content, tokens, diff_bg, cursor, room));
        // Line endings only matter where they changed.
        if crlf && line.kind != LineKind::Context {
            spans.push(Span::styled("␍", theme.meta(bg)));
        }
        Line::from(spans).render(area, buf);
    }
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
    let bg = if cursor {
        Bg::DiffSelected(diff_bg)
    } else {
        Bg::Diff(diff_bg)
    };
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
