//! Changed files as a directory tree. Single-child directory chains are
//! compressed (`src/ui/` rather than `src/` then `ui/`), and rows follow the
//! diff's file order so `]f` and the tree agree.

#![deny(clippy::arithmetic_side_effects)]

use std::collections::HashSet;

use crate::git::files::FileStatus;
use crate::theme::{Bg, Fg};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::ui::diff_doc::{Doc, Viewed};
use crate::ui::{Ctx, PAD_Y, fill, fit_spans, inset, render_split, text};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRow {
    Dir {
        name: String,
        /// The whole path, which a fold is kept by.
        path: String,
        depth: u16,
        folded: bool,
    },
    File {
        index: usize,
        name: String,
        depth: u16,
    },
}

#[derive(Default)]
struct Node {
    name: String,
    file: Option<usize>,
    children: Vec<Node>,
}

pub fn tree_rows(doc: &Doc) -> Vec<TreeRow> {
    let mut root = Node::default();
    for (index, file) in doc.files.iter().enumerate() {
        let mut node = &mut root;
        let mut parts = file.meta.path().split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                node.children.push(Node {
                    name: part.to_owned(),
                    file: Some(index),
                    children: Vec::new(),
                });
                break;
            }
            let pos = node
                .children
                .iter()
                .position(|c| c.file.is_none() && c.name == part)
                .unwrap_or_else(|| {
                    node.children.push(Node {
                        name: part.to_owned(),
                        ..Node::default()
                    });
                    node.children.len().saturating_sub(1)
                });
            let Some(child) = node.children.get_mut(pos) else {
                break;
            };
            node = child;
        }
    }
    let mut rows = Vec::new();
    flatten(&root, "", 0, &mut rows);
    rows
}

impl TreeRow {
    pub fn depth(&self) -> u16 {
        match self {
            TreeRow::Dir { depth, .. } | TreeRow::File { depth, .. } => *depth,
        }
    }
}

/// `rows` with the folders in `folded` folded: what's in them is hidden.
pub fn unfolded(rows: &[TreeRow], folded: &HashSet<String>) -> Vec<TreeRow> {
    let mut out = Vec::new();
    // The depth of a folded folder being skipped.
    let mut hiding: Option<u16> = None;
    for row in rows {
        let depth = row.depth();
        if hiding.is_some_and(|d| depth > d) {
            continue;
        }
        hiding = None;
        let mut row = row.clone();
        if let TreeRow::Dir {
            path, folded: f, ..
        } = &mut row
            && folded.contains(path.as_str())
        {
            *f = true;
            hiding = Some(depth);
        }
        out.push(row);
    }
    out
}

fn flatten(node: &Node, parent: &str, depth: u16, rows: &mut Vec<TreeRow>) {
    for child in &node.children {
        if let Some(index) = child.file {
            rows.push(TreeRow::File {
                index,
                name: child.name.clone(),
                depth,
            });
            continue;
        }
        let mut dir = child;
        let mut name = child.name.clone();
        while let [only] = dir.children.as_slice()
            && only.file.is_none()
        {
            dir = only;
            name = format!("{name}/{}", dir.name);
        }
        let path = if parent.is_empty() {
            name.clone()
        } else {
            format!("{parent}/{name}")
        };
        rows.push(TreeRow::Dir {
            name: format!("{name}/"),
            path: path.clone(),
            depth,
            folded: false,
        });
        flatten(dir, &path, depth.saturating_add(1), rows);
    }
}

/// Tree row showing file `index`.
pub fn row_of_file(rows: &[TreeRow], index: usize) -> Option<usize> {
    rows.iter()
        .position(|r| matches!(r, TreeRow::File { index: i, .. } if *i == index))
}

pub struct FileTree<'a> {
    pub ctx: Ctx<'a>,
    pub doc: &'a Doc,
    pub rows: &'a [TreeRow],
    pub selected: usize,
    pub scroll: usize,
    pub focused: bool,
}

pub const TREE_BG: Bg = Bg::ContainerLow;

impl Widget for FileTree<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        fill(buf, area, theme, TREE_BG);
        if area.height <= PAD_Y {
            return;
        }
        let title_area = Rect {
            y: area
                .y
                .saturating_add(PAD_Y.min(area.height.saturating_sub(1))),
            height: 1,
            ..inset(area, 1, 0)
        };
        let (adds, dels) = self.doc.totals();
        let title = [
            Span::styled("Files", theme.title(TREE_BG)),
            Span::styled(format!("  {}", self.doc.files.len()), theme.meta(TREE_BG)),
            Span::styled(
                match self.doc.viewed_count() {
                    0 => String::new(),
                    n => format!("  {n} viewed"),
                },
                theme.style(Fg::Success, TREE_BG),
            ),
            Span::styled(
                format!("  +{adds}"),
                theme.style(Fg::DiffAddedSign, TREE_BG),
            ),
            Span::styled(
                format!(" −{dels}"),
                theme.style(Fg::DiffRemovedSign, TREE_BG),
            ),
        ];
        Line::from(fit_spans(&title, title_area.width)).render(title_area, buf);

        let list_top = title_area.y.saturating_add(2);
        let shown = self.rows.iter().enumerate().skip(self.scroll);
        for ((i, row), y) in shown.zip(list_top..area.bottom()) {
            let selected = i == self.selected;
            let bg = match (selected, self.focused) {
                (true, true) => Bg::Selected,
                (true, false) => Bg::SelectedInactive,
                _ => TREE_BG,
            };
            let row_area = Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            };
            if selected {
                fill(buf, row_area, theme, bg);
            }
            self.render_row(row, bg, inset(row_area, 1, 0), buf);
        }
    }
}

impl FileTree<'_> {
    fn render_row(&self, row: &TreeRow, bg: Bg, area: Rect, buf: &mut Buffer) {
        let theme = self.ctx.theme;
        match row {
            TreeRow::Dir {
                name,
                depth,
                folded,
                ..
            } => {
                let indent = " ".repeat(usize::from(*depth) * 2);
                let room = usize::from(area.width).saturating_sub(indent.len().saturating_add(2));
                let fold = if *folded { "▸ " } else { "▾ " };
                Line::from(vec![
                    Span::styled(indent, theme.body(bg)),
                    Span::styled(fold, theme.meta(bg)),
                    Span::styled(text::truncate(name, room), theme.meta(bg)),
                ])
                .render(area, buf);
            }
            TreeRow::File { index, name, depth } => {
                let Some(file) = self.doc.files.get(*index) else {
                    return;
                };
                let (mark, mark_fg) = match file.meta.status {
                    FileStatus::Added => ("A", Fg::Success),
                    FileStatus::Deleted => ("D", Fg::Error),
                    FileStatus::Renamed | FileStatus::Copied => ("R", Fg::Tertiary),
                    FileStatus::TypeChanged => ("T", Fg::OnSurfaceVariant),
                    FileStatus::Modified => ("M", Fg::OnSurfaceVariant),
                };
                let counts = self.doc.file_counts(file);
                let right = match file.diff.as_deref() {
                    Some(_) => vec![
                        Span::styled(format!("+{}", counts.0), theme.style(Fg::DiffAddedSign, bg)),
                        Span::styled(
                            format!(" −{}", counts.1),
                            theme.style(Fg::DiffRemovedSign, bg),
                        ),
                    ],
                    None => Vec::new(),
                };
                let right_width = text::spans_width(&right);
                let indent = " ".repeat(usize::from(*depth) * 2);
                let room = usize::from(area.width)
                    .saturating_sub(indent.len())
                    .saturating_sub(right_width.saturating_add(3));
                let viewed = file.viewed == Viewed::Viewed;
                let name_style = if file.generated || viewed {
                    theme.meta(bg)
                } else {
                    theme.body(bg)
                };
                let mut spans = vec![
                    Span::styled(indent, theme.body(bg)),
                    Span::styled(mark, theme.style(mark_fg, bg)),
                    Span::styled(" ", theme.body(bg)),
                ];
                let room = if viewed {
                    spans.push(Span::styled("✓ ", theme.style(Fg::Success, bg)));
                    room.saturating_sub(2)
                } else {
                    room
                };
                spans.push(Span::styled(text::truncate(name, room), name_style));
                render_split(area, buf, &spans, &right, 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::diff_doc::tests::changed;

    fn doc(paths: &[&str]) -> Doc {
        Doc::new(
            paths.iter().map(|p| changed(p)).collect(),
            &HashSet::new(),
            Default::default(),
        )
    }

    fn render(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                TreeRow::Dir {
                    name,
                    depth,
                    folded,
                    ..
                } => {
                    let fold = if *folded { "+" } else { "" };
                    format!("{}{fold}{name}", "  ".repeat(*depth as usize))
                }
                TreeRow::File { name, depth, index } => {
                    format!("{}{name} #{index}", "  ".repeat(*depth as usize))
                }
            })
            .collect()
    }

    #[test]
    fn builds_compressed_tree_in_file_order() {
        let doc = doc(&[
            "Cargo.toml",
            "crates/app/src/main.rs",
            "crates/app/src/state.rs",
            "crates/ui/src/lib.rs",
            "docs/guide/intro.md",
        ]);
        assert_eq!(
            render(&tree_rows(&doc)),
            [
                "Cargo.toml #0",
                "crates/",
                "  app/src/",
                "    main.rs #1",
                "    state.rs #2",
                "  ui/src/",
                "    lib.rs #3",
                "docs/guide/",
                "  intro.md #4",
            ]
        );
    }

    #[test]
    fn folded_folders_hide_what_is_in_them() {
        let doc = doc(&["a/x/1.rs", "a/x/2.rs", "a/3.rs", "b.rs"]);
        let folded = HashSet::from(["a/x".to_owned()]);
        assert_eq!(
            render(&unfolded(&tree_rows(&doc), &folded)),
            ["a/", "  +x/", "  3.rs #2", "b.rs #3"]
        );
    }

    #[test]
    fn finds_file_rows() {
        let doc = doc(&["a/b.rs", "c.rs"]);
        let rows = tree_rows(&doc);
        assert_eq!(row_of_file(&rows, 0), Some(1));
        assert_eq!(row_of_file(&rows, 1), Some(2));
        assert_eq!(row_of_file(&rows, 9), None);
    }
}
