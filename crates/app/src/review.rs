//! Reviewing: comment targets, drafts, suggestions, and turning GitHub
//! threads and local drafts into annotations.

use std::collections::HashMap;

use ghtui_api::model::{NewThread, ReviewEvent, ReviewThread, Side as ApiSide};
use ghtui_diff::anchor::{AnchorError, LinePos, Side};
use ghtui_store::{DraftComment, DraftSide};
use ghtui_theme::{Bg, Theme};
use ghtui_ui::annotations::{Annotation, AnnotationComment, AnnotationKey};
use ghtui_ui::diff_doc::{Doc, Pos};
use ratatui_textarea::TextArea;

/// What a comment being written will be attached to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposeTarget {
    /// One line (`start == end`) or a range, by file line numbers.
    Line {
        path: String,
        start: LinePos,
        end: LinePos,
    },
    /// The whole file; `reason` says why when a line was asked for.
    File {
        path: String,
        reason: Option<String>,
    },
    /// A reply, posted right away.
    Reply { thread_id: String },
    /// Editing an existing draft.
    Draft { id: u64 },
    /// A comment on an issue or pull request's conversation, posted right
    /// away; `refresh` is the page data to reload after.
    Conversation {
        subject_id: String,
        name: String,
        refresh: crate::browse::DataKey,
    },
}

/// A suggested change and what it replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub start_line: u32,
    pub original: Vec<String>,
    pub suggested: Vec<String>,
}

pub struct Compose {
    pub target: ComposeTarget,
    pub input: TextArea<'static>,
    pub preview: Option<Preview>,
    /// A reply is being posted.
    pub sending: bool,
    pub error: Option<String>,
    /// Esc was pressed once with text in the editor.
    pub confirm_discard: bool,
}

impl Compose {
    pub fn new(theme: &Theme, target: ComposeTarget, text: &str) -> Self {
        Self {
            target,
            input: editor(theme, text),
            preview: None,
            sending: false,
            error: None,
            confirm_discard: false,
        }
    }

    pub fn text(&self) -> String {
        self.input.lines().join("\n")
    }

    pub fn title(&self) -> String {
        let side = |s: Side| match s {
            Side::Left => "old",
            Side::Right => "new",
        };
        match &self.target {
            ComposeTarget::Line { path, start, end } if start == end => {
                format!("Comment on {path}:{} ({})", end.line, side(end.side))
            }
            ComposeTarget::Line { path, start, end } => format!(
                "Comment on {path}, lines {}–{} ({})",
                start.line,
                end.line,
                side(end.side)
            ),
            ComposeTarget::File { path, .. } => format!("Comment on the file {path}"),
            ComposeTarget::Reply { .. } => "Reply".to_owned(),
            ComposeTarget::Draft { .. } => "Edit draft".to_owned(),
            ComposeTarget::Conversation { name, .. } => format!("Comment on {name}"),
        }
    }
}

pub struct SubmitDialog {
    pub event: ReviewEvent,
    pub input: TextArea<'static>,
    pub sending: bool,
    pub error: Option<String>,
}

impl SubmitDialog {
    pub fn new(theme: &Theme) -> Self {
        let mut input = editor(theme, "");
        input.set_placeholder_text("Summary (optional for comments and approvals)");
        input.set_placeholder_style(theme.meta(Bg::ContainerHigh));
        Self {
            event: ReviewEvent::Comment,
            input,
            sending: false,
            error: None,
        }
    }

    pub fn cycle(&mut self, forward: bool) {
        const ORDER: [ReviewEvent; 3] = [
            ReviewEvent::Comment,
            ReviewEvent::Approve,
            ReviewEvent::RequestChanges,
        ];
        let i = ORDER.iter().position(|e| *e == self.event).unwrap_or(0);
        let next = if forward { i + 1 } else { i + ORDER.len() - 1 };
        self.event = ORDER[next % ORDER.len()];
    }
}

fn editor(theme: &Theme, text: &str) -> TextArea<'static> {
    let mut input = crate::nav::new_input(theme, Bg::ContainerHigh);
    input.insert_str(text.trim_end_matches('\n'));
    input
}

/// Why `$EDITOR` was opened, and where its text goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditPurpose {
    /// Continue the comment being composed.
    Compose,
    /// A suggested change for these new-side lines.
    Suggest {
        path: String,
        start: LinePos,
        end: LinePos,
        original: Vec<String>,
    },
    /// The review summary.
    Summary,
}

/// What happened on submit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SubmitOutcome {
    /// Drafts GitHub accepted (now in the pending review there).
    pub accepted: Vec<u64>,
    /// Drafts GitHub rejected, with its reason.
    pub rejected: Vec<(u64, String)>,
    /// The review was submitted.
    pub submitted: bool,
    /// The submission as a whole failed.
    pub error: Option<String>,
}

pub fn suggestion_body(suggested: &str) -> String {
    format!("```suggestion\n{}\n```", suggested.trim_end_matches('\n'))
}

/// Where a new comment goes, from the cursor or a visual selection. Lines
/// GitHub won't accept become a file comment (with the reason); a range that
/// crosses GitHub's hunks is refused.
pub fn target(doc: &Doc, cursor: Pos, selection: Option<Pos>) -> Result<ComposeTarget, String> {
    let file = doc.files.get(cursor.file).ok_or("No file here")?;
    let path = file.meta.path().to_owned();
    let (from, to) = match selection {
        Some(anchor) if anchor.file == cursor.file => {
            let (a, b) = (doc.to_global(anchor), doc.to_global(cursor));
            (a.min(b), a.max(b))
        }
        Some(_) => return Err("A selection can't span files".into()),
        None => {
            let g = doc.to_global(cursor);
            (g, g)
        }
    };
    let lines: Vec<LinePos> = (from..=to)
        .filter_map(|g| doc.line_at(doc.to_pos(g)))
        .collect();
    let (Some(&start), Some(&end)) = (lines.first(), lines.last()) else {
        return Ok(ComposeTarget::File { path, reason: None });
    };
    let Some(commentable) = doc.commentable(cursor.file) else {
        return Err("This file's diff hasn't loaded yet".into());
    };
    match commentable.validate(start, end) {
        Ok(()) => Ok(ComposeTarget::Line { path, start, end }),
        Err(AnchorError::NotCommentable) => Ok(ComposeTarget::File {
            path,
            reason: Some(
                "That line isn't part of GitHub's diff, so this will be a comment on the file."
                    .into(),
            ),
        }),
        Err(err) => Err(format!("Can't comment there: {err}")),
    }
}

fn draft_side(side: Side) -> DraftSide {
    match side {
        Side::Left => DraftSide::Left,
        Side::Right => DraftSide::Right,
    }
}

fn side_of(side: DraftSide) -> Side {
    match side {
        DraftSide::Left => Side::Left,
        DraftSide::Right => Side::Right,
    }
}

/// A new draft for a line or file target.
pub fn draft(target: &ComposeTarget, body: String, id: u64, commit: &str) -> Option<DraftComment> {
    let base = |path: &str| DraftComment {
        id,
        path: path.to_owned(),
        body: body.clone(),
        side: DraftSide::Right,
        line: None,
        start_line: None,
        start_side: None,
        commit: commit.to_owned(),
        error: None,
    };
    match target {
        ComposeTarget::Line { path, start, end } => {
            let ranged = start != end;
            Some(DraftComment {
                side: draft_side(end.side),
                line: Some(end.line),
                start_line: ranged.then_some(start.line),
                start_side: ranged.then_some(draft_side(start.side)),
                ..base(path)
            })
        }
        ComposeTarget::File { path, .. } => Some(base(path)),
        ComposeTarget::Reply { .. }
        | ComposeTarget::Draft { .. }
        | ComposeTarget::Conversation { .. } => None,
    }
}

pub fn new_thread(draft: &DraftComment) -> NewThread {
    let side = |s: DraftSide| match s {
        DraftSide::Left => ApiSide::Left,
        DraftSide::Right => ApiSide::Right,
    };
    NewThread {
        path: draft.path.clone(),
        body: draft.body.clone(),
        line: draft.line,
        side: side(draft.side),
        start_line: draft.start_line,
        start_side: draft.start_side.map(side),
    }
}

/// GitHub threads and local drafts as annotations. Outdated threads whose
/// line was mapped forward sit on their new line; unmapped ones go under the
/// file header.
pub fn annotations(
    threads: &[ReviewThread],
    mapped: &HashMap<String, Option<u32>>,
    drafts: &[DraftComment],
) -> Vec<Annotation> {
    let mut out: Vec<Annotation> = threads
        .iter()
        .map(|t| {
            let side = match t.side {
                ApiSide::Left => Side::Left,
                ApiSide::Right => Side::Right,
            };
            let moved_to = mapped.get(&t.id).copied().flatten();
            let (line, outdated, moved) = match (t.outdated, t.line, moved_to) {
                (false, Some(line), _) => (Some(line), false, false),
                (_, _, Some(line)) => (Some(line), false, true),
                (_, line, None) => (line, t.outdated, false),
            };
            Annotation {
                key: AnnotationKey::Thread(t.id.clone()),
                path: t.path.clone(),
                side,
                line,
                start_line: if moved { None } else { t.start_line },
                original_line: t.original_line,
                resolved: t.resolved,
                outdated,
                moved,
                file_level: t.file_level,
                comments: t
                    .comments
                    .iter()
                    .map(|c| AnnotationComment {
                        author: c.author.clone(),
                        body: c.body.clone(),
                        created_at: c.created_at.clone(),
                        pending: c.pending,
                    })
                    .collect(),
                error: None,
                can_reply: t.can_reply,
                can_resolve: t.can_resolve,
                can_unresolve: t.can_unresolve,
            }
        })
        .collect();
    out.extend(drafts.iter().map(|d| Annotation {
        key: AnnotationKey::Draft(d.id),
        path: d.path.clone(),
        side: side_of(d.side),
        line: d.line,
        start_line: d.start_line,
        original_line: d.line,
        resolved: false,
        outdated: false,
        moved: false,
        file_level: d.line.is_none(),
        comments: vec![AnnotationComment {
            author: "You".into(),
            body: d.body.clone(),
            created_at: String::new(),
            pending: false,
        }],
        error: d.error.clone(),
        can_reply: false,
        can_resolve: false,
        can_unresolve: false,
    }));
    out
}

/// Outdated right-side threads worth mapping: `(thread, path, commit, line)`.
pub fn outdated_to_map(threads: &[ReviewThread]) -> Vec<(String, String, String, u32)> {
    threads
        .iter()
        .filter(|t| t.outdated && !t.file_level && t.side == ApiSide::Right)
        .filter_map(|t| {
            let commit = t.comments.first()?.original_commit.clone()?;
            Some((t.id.clone(), t.path.clone(), commit, t.original_line?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ghtui_api::model::ReviewComment;

    fn thread(id: &str, outdated: bool, line: Option<u32>) -> ReviewThread {
        ReviewThread {
            id: id.into(),
            path: "a.rs".into(),
            side: ApiSide::Right,
            start_side: None,
            line,
            start_line: None,
            original_line: Some(7),
            original_start_line: None,
            outdated,
            resolved: false,
            file_level: false,
            can_reply: true,
            can_resolve: true,
            can_unresolve: false,
            comments: vec![ReviewComment {
                id: "c".into(),
                author: "alice".into(),
                body: "hm".into(),
                created_at: "2026-10-01T00:00:00Z".into(),
                url: String::new(),
                original_commit: Some("abc".into()),
                pending: false,
            }],
        }
    }

    #[test]
    fn outdated_threads_move_when_mapped() {
        let threads = vec![
            thread("current", false, Some(3)),
            thread("mapped", true, None),
            thread("lost", true, None),
        ];
        let mapped = HashMap::from([("mapped".to_owned(), Some(12)), ("lost".to_owned(), None)]);
        let anns = annotations(&threads, &mapped, &[]);
        assert_eq!((anns[0].line, anns[0].outdated), (Some(3), false));
        assert_eq!(
            (anns[1].line, anns[1].moved, anns[1].outdated),
            (Some(12), true, false)
        );
        assert!(anns[2].outdated && anns[2].on_line().is_none());
        assert_eq!(outdated_to_map(&threads).len(), 2);
    }

    #[test]
    fn drafts_become_ranges_or_file_comments() {
        let right = |line| LinePos {
            side: Side::Right,
            line,
        };
        let range = ComposeTarget::Line {
            path: "a.rs".into(),
            start: right(3),
            end: right(5),
        };
        let d = draft(&range, "x".into(), 1, "head").unwrap();
        assert_eq!(
            (d.line, d.start_line, d.start_side),
            (Some(5), Some(3), Some(DraftSide::Right))
        );
        let thread = new_thread(&d);
        assert_eq!((thread.line, thread.start_line), (Some(5), Some(3)));

        let single = ComposeTarget::Line {
            path: "a.rs".into(),
            start: right(4),
            end: right(4),
        };
        let d = draft(&single, "x".into(), 2, "head").unwrap();
        assert_eq!(
            (d.start_line, d.start_side),
            (None, None),
            "single lines have no start"
        );

        let file = ComposeTarget::File {
            path: "a.rs".into(),
            reason: None,
        };
        assert_eq!(draft(&file, "x".into(), 3, "head").unwrap().line, None);
        assert!(
            draft(
                &ComposeTarget::Reply {
                    thread_id: "t".into()
                },
                "x".into(),
                4,
                "h"
            )
            .is_none()
        );

        let anns = annotations(&[], &HashMap::new(), &[d_with_error()]);
        assert!(anns[0].is_draft());
        assert_eq!(anns[0].error.as_deref(), Some("nope"));
    }

    fn d_with_error() -> DraftComment {
        DraftComment {
            id: 9,
            path: "a.rs".into(),
            body: "b".into(),
            side: DraftSide::Right,
            line: Some(1),
            start_line: None,
            start_side: None,
            commit: "h".into(),
            error: Some("nope".into()),
        }
    }

    #[test]
    fn suggestion_fences() {
        assert_eq!(
            suggestion_body("let x = 1;\n"),
            "```suggestion\nlet x = 1;\n```"
        );
    }

    #[test]
    fn submit_events_cycle() {
        let theme = Theme::new(
            ghtui_theme::DEFAULT_SEED,
            ghtui_theme::Mode::Dark,
            ghtui_theme::ColorDepth::TrueColor,
        );
        let mut dialog = SubmitDialog::new(&theme);
        dialog.cycle(true);
        assert_eq!(dialog.event, ReviewEvent::Approve);
        dialog.cycle(true);
        assert_eq!(dialog.event, ReviewEvent::RequestChanges);
        dialog.cycle(true);
        assert_eq!(dialog.event, ReviewEvent::Comment);
        dialog.cycle(false);
        assert_eq!(dialog.event, ReviewEvent::RequestChanges);
    }
}
