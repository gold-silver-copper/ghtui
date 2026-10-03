//! The diff screen: file tree plus unified diff, and its key handling.

use std::collections::HashSet;
use std::sync::Arc;

use ghtui_api::model::PrRef;
use ghtui_diff::FileDiff;
use ghtui_git::repo::PrRefs;
use ghtui_ui::diff_doc::{Doc, Note, Pos, Row};
use ghtui_ui::file_tree::{TreeRow, row_of_file, tree_rows};
use ratatui::layout::Rect;

use crate::keymap::Action;
use crate::state::Cmd;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Tree,
    Diff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffScreen {
    pub pr: PrRef,
    pub cursor: Pos,
    pub top: Pos,
    pub tree_visible: bool,
    pub focus: Pane,
    pub tree_selected: usize,
    pub tree_scroll: usize,
}

impl DiffScreen {
    pub fn new(pr: PrRef, width: u16) -> Self {
        Self {
            pr,
            cursor: Pos::default(),
            top: Pos::default(),
            tree_visible: width >= 100,
            focus: Pane::Diff,
            tree_selected: 0,
            tree_scroll: 0,
        }
    }
}

/// Everything known about one PR's diff.
#[derive(Debug, Default)]
pub struct DiffState {
    pub doc: Doc,
    pub tree: Vec<TreeRow>,
    pub refs: Option<PrRefs>,
    /// What the background job is doing, while it works.
    pub progress: Option<String>,
    pub error: Option<String>,
    /// The file list has arrived.
    pub listed: bool,
    /// Files we've already asked the job to prioritize.
    requested: HashSet<usize>,
}

impl DiffState {
    pub fn loading() -> Self {
        Self {
            progress: Some("Preparing".into()),
            ..Self::default()
        }
    }

    pub fn set_files(&mut self, refs: PrRefs, doc: Doc) {
        self.tree = tree_rows(&doc);
        self.doc = doc;
        self.refs = Some(refs);
        self.listed = true;
        self.progress = if self.doc.is_empty() {
            None
        } else {
            Some("Computing diffs".into())
        };
    }

    pub fn set_file(&mut self, index: usize, diff: Arc<FileDiff>) {
        self.doc.set_diff(index, diff);
        if self.doc.ready_count() == self.doc.files.len() {
            self.progress = None;
        }
    }

    pub fn status(&self) -> Option<String> {
        let progress = self.progress.as_ref()?;
        if self.listed && !self.doc.is_empty() {
            Some(format!(
                "Diffing {}/{} files",
                self.doc.ready_count(),
                self.doc.files.len()
            ))
        } else {
            Some(progress.clone())
        }
    }
}

pub struct Layout {
    pub tree: Option<Rect>,
    pub diff: Rect,
}

/// Splits the content area; shared by the view and key handling so both
/// agree on viewport sizes.
pub fn layout(content: Rect, tree_visible: bool) -> Layout {
    if !tree_visible || content.width < 60 {
        return Layout {
            tree: None,
            diff: content,
        };
    }
    let width = (content.width / 4).clamp(24, 40);
    Layout {
        tree: Some(Rect { width, ..content }),
        diff: Rect {
            x: content.x + width,
            width: content.width - width,
            ..content
        },
    }
}

/// Rows of the tree list below its title.
fn tree_list_height(tree: Rect) -> usize {
    usize::from(tree.height.saturating_sub(3))
}

/// Handles `action` on the diff screen. `None` means it isn't a diff-screen
/// action (the caller handles it).
pub fn apply(
    screen: &mut DiffScreen,
    state: &mut DiffState,
    action: Action,
    content: Rect,
) -> Option<Vec<Cmd>> {
    let lay = layout(content, screen.tree_visible);
    let doc = &state.doc;
    let half = (usize::from(lay.diff.height) / 2).max(1) as isize;
    match action {
        Action::ToggleTree => {
            screen.tree_visible = !screen.tree_visible;
            if !screen.tree_visible {
                screen.focus = Pane::Diff;
            }
        }
        Action::SwitchPane => {
            if lay.tree.is_some() {
                screen.focus = match screen.focus {
                    Pane::Tree => Pane::Diff,
                    Pane::Diff => Pane::Tree,
                };
            }
        }
        _ if screen.focus == Pane::Tree && lay.tree.is_some() => {
            let tree_half = (tree_list_height(lay.tree.unwrap_or_default()) / 2).max(1) as isize;
            let delta = match action {
                Action::Down => 1,
                Action::Up => -1,
                Action::HalfPageDown => tree_half,
                Action::HalfPageUp => -tree_half,
                Action::Top => isize::MIN / 2,
                Action::Bottom => isize::MAX / 2,
                Action::Open => {
                    screen.focus = Pane::Diff;
                    return Some(settle(screen, state, content));
                }
                _ => return None,
            };
            let last = state.tree.len().saturating_sub(1) as isize;
            screen.tree_selected = (screen.tree_selected as isize)
                .saturating_add(delta)
                .clamp(0, last) as usize;
            // The diff follows the tree selection.
            if let Some(TreeRow::File { index, .. }) = state.tree.get(screen.tree_selected) {
                screen.cursor = Pos {
                    file: *index,
                    row: 0,
                };
                screen.top = screen.cursor;
            }
        }
        Action::Down => screen.cursor = doc.offset(screen.cursor, 1),
        Action::Up => screen.cursor = doc.offset(screen.cursor, -1),
        Action::HalfPageDown => {
            screen.cursor = doc.offset(screen.cursor, half);
            screen.top = doc.offset(screen.top, half);
        }
        Action::HalfPageUp => {
            screen.cursor = doc.offset(screen.cursor, -half);
            screen.top = doc.offset(screen.top, -half);
        }
        Action::Top => screen.cursor = Pos::default(),
        Action::Bottom => screen.cursor = doc.last(),
        Action::NextHunk => {
            if let Some(pos) = doc.next_hunk(screen.cursor) {
                screen.cursor = pos;
            }
        }
        Action::PrevHunk => {
            if let Some(pos) = doc.prev_hunk(screen.cursor) {
                screen.cursor = pos;
            }
        }
        Action::NextFile => {
            if let Some(pos) = doc.next_file(screen.cursor) {
                screen.cursor = pos;
                screen.top = pos;
            }
        }
        Action::PrevFile => {
            if let Some(pos) = doc.prev_file(screen.cursor) {
                screen.cursor = pos;
                screen.top = pos;
            }
        }
        Action::Open => {
            let on_collapsed = matches!(
                doc.row(screen.cursor),
                Some(Row::Header | Row::Note(Note::Collapsed))
            ) && doc
                .files
                .get(screen.cursor.file)
                .is_some_and(|f| f.generated);
            if !on_collapsed {
                return Some(Vec::new());
            }
            let file = screen.cursor.file;
            state.doc.toggle_expanded(file);
            // Expanded files are diffed on demand.
            state.requested.remove(&file);
        }
        _ => return None,
    }
    Some(settle(screen, state, content))
}

/// Clamps positions, keeps the cursor visible (clear of the sticky header),
/// syncs the tree with the diff, and asks the job to prioritize files that
/// are on screen but not diffed yet.
pub fn settle(screen: &mut DiffScreen, state: &mut DiffState, content: Rect) -> Vec<Cmd> {
    let lay = layout(content, screen.tree_visible);
    if lay.tree.is_none() {
        screen.focus = Pane::Diff;
    }
    let doc = &state.doc;
    if doc.is_empty() {
        return Vec::new();
    }
    screen.cursor = doc.clamp(screen.cursor);
    let height = usize::from(lay.diff.height).max(1);
    let total = doc.total_rows();
    let cursor = doc.to_global(screen.cursor);
    let mut top = doc.to_global(screen.top).min(total.saturating_sub(height));
    // Row 0 of the viewport is covered by the sticky header unless it's the
    // file's own header.
    let upper = if screen.cursor.row == 0 {
        cursor
    } else {
        cursor.saturating_sub(1)
    };
    if top > upper {
        top = upper;
    }
    if cursor >= top + height {
        top = cursor + 1 - height;
    }
    screen.top = doc.to_pos(top);

    if screen.focus == Pane::Diff
        && let Some(row) = row_of_file(&state.tree, screen.cursor.file)
    {
        screen.tree_selected = row;
    }
    if let Some(tree) = lay.tree {
        let rows = tree_list_height(tree).max(1);
        if screen.tree_selected < screen.tree_scroll {
            screen.tree_scroll = screen.tree_selected;
        } else if screen.tree_selected >= screen.tree_scroll + rows {
            screen.tree_scroll = screen.tree_selected + 1 - rows;
        }
    }

    let mut wanted = vec![screen.cursor.file];
    wanted.extend(doc.loading_in_view(screen.top, height));
    let wanted: Vec<usize> = wanted
        .into_iter()
        .filter(|f| {
            doc.files[*f].diff.is_none()
                && !doc.files[*f].collapsed()
                && !state.requested.contains(f)
        })
        .collect();
    if wanted.is_empty() {
        return Vec::new();
    }
    state.requested.extend(wanted.iter().copied());
    vec![Cmd::Prioritize(screen.pr.clone(), wanted)]
}
