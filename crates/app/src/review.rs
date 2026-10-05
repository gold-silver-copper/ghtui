//! Reviewing: comment targets, drafts, suggestions, and turning GitHub
//! threads and local drafts into annotations.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ghtui_api::model::{NewThread, ReviewEvent, ReviewThread, Side as ApiSide};
use ghtui_api::model::{NodeId, PrRef};
use ghtui_diff::anchor::{AnchorError, LinePos, Side};
use ghtui_store::{DraftComment, DraftSide};
use ghtui_theme::{Bg, Theme};
use ghtui_ui::annotations::{Annotation, AnnotationComment, AnnotationKey};
use ghtui_ui::bars::Notice;
use ghtui_ui::diff_doc::{Doc, Pos};
use ghtui_ui::text::short_sha;
use ratatui_textarea::TextArea;

use crate::diff_screen::{self, DiffScreen, LastReview};
use crate::keymap::Action;
use crate::picker::{self, PickItem};
use crate::state::{Api, Cmd, Git, OutdatedThread, Overlay, Screen, State};

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
    Reply { thread_id: NodeId },
    /// Editing an existing draft.
    Draft { id: u64 },
    /// A comment on an issue or pull request's conversation, posted right
    /// away; `refresh` is the page data to reload after.
    Conversation {
        subject_id: NodeId,
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
        let steps = if forward { 1 } else { ORDER.len() - 1 };
        if let Some(&next) = ORDER.iter().cycle().nth(i + steps) {
            self.event = next;
        }
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
    let file = doc.files().get(cursor.file).ok_or("No file here")?;
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
pub fn draft(target: &ComposeTarget, body: &str, id: u64, commit: &str) -> Option<DraftComment> {
    let base = |path: &str| DraftComment {
        id,
        path: path.to_owned(),
        body: body.to_owned(),
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
    mapped: &HashMap<NodeId, Option<u32>>,
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

/// Outdated right-side threads worth mapping onto the current diff.
pub fn outdated_to_map(threads: &[ReviewThread]) -> Vec<OutdatedThread> {
    threads
        .iter()
        .filter(|t| t.outdated && !t.file_level && t.side == ApiSide::Right)
        .filter_map(|t| {
            Some(OutdatedThread {
                thread: t.id.clone(),
                path: t.path.clone(),
                commit: t.comments.first()?.original_commit.clone()?,
                line: t.original_line?,
            })
        })
        .collect()
}

/// Applies the result of submitting a review: accepted drafts leave the
/// local queue, rejected ones keep their text and GitHub's reason.
#[must_use]
pub(crate) fn on_submitted(state: &mut State, pr: &PrRef, outcome: &SubmitOutcome) -> Vec<Cmd> {
    let mut cmds = vec![Cmd::Api(Api::FetchThreads(pr.clone()))];
    if let Some(diff) = state.diffs.get_mut(pr) {
        diff.review
            .pending
            .retain(|d| !outcome.accepted.contains(&d.id));
        for draft in &mut diff.review.pending {
            draft.error = outcome
                .rejected
                .iter()
                .find(|(id, _)| *id == draft.id)
                .map(|(_, reason)| reason.clone());
        }
        if outcome.submitted {
            diff.review.last_reviewed_head = diff.head().map(|h| h.to_string());
        }
        diff.refresh_annotations();
        cmds.push(Cmd::SaveReview(pr.clone(), diff.review.clone()));
    }
    if outcome.submitted {
        state.overlay = None;
        state.info("Review submitted");
    } else if let Some(Overlay::Submit(dialog)) = &mut state.overlay {
        dialog.sending = false;
        dialog.error = Some(match (&outcome.error, outcome.rejected.len()) {
            (Some(err), _) => err.clone(),
            (None, n) => format!(
                "GitHub rejected {n} comment{}; they're marked in the diff. The review stays pending on GitHub until you submit again.",
                if n == 1 { "" } else { "s" }
            ),
        });
    }
    cmds
}

#[must_use]
pub(crate) fn on_edited(
    state: &mut State,
    purpose: EditPurpose,
    result: Result<String, crate::state::Failure>,
) -> Vec<Cmd> {
    let text = match result {
        Ok(text) => text,
        Err(err) => {
            state.error(format!("Couldn't run the editor: {err}"));
            return Vec::new();
        }
    };
    let theme = &state.theme;
    match purpose {
        EditPurpose::Compose => {
            if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                let target = compose.target.clone();
                let preview = compose.preview.clone();
                **compose = Compose::new(theme, target, &text);
                compose.preview = preview;
            }
        }
        EditPurpose::Summary => {
            if let Some(Overlay::Submit(dialog)) = &mut state.overlay {
                let event = dialog.event;
                **dialog = SubmitDialog::new(theme);
                dialog.event = event;
                dialog.input.insert_str(text.trim_end());
            }
        }
        EditPurpose::Suggest {
            path,
            start,
            end,
            original,
        } => {
            let suggested: Vec<String> = text
                .trim_end_matches('\n')
                .lines()
                .map(str::to_owned)
                .collect();
            if suggested == original {
                state.info("No change suggested");
                return Vec::new();
            }
            let body = suggestion_body(&suggested.join("\n"));
            let mut compose = Compose::new(theme, ComposeTarget::Line { path, start, end }, &body);
            compose.preview = Some(Preview {
                start_line: start.line,
                original,
                suggested,
            });
            state.overlay = Some(Overlay::Compose(Box::new(compose)));
        }
    }
    Vec::new()
}

/// How to get back to the whole PR, for notices.
pub(crate) fn all_changes(state: &State) -> String {
    let key = state.first_key(Action::PickCommits);
    format!("pick “All changes” ({key})")
}

/// Review actions on the diff screen. `None` lets other handlers try.
#[must_use]
pub(crate) fn review_action(state: &mut State, action: Action) -> Option<Vec<Cmd>> {
    let viewer = state.viewer.clone();
    let (screen, diff) = state.diff_parts()?;
    let pr = screen.pr.clone();
    let cursor = screen.cursor;
    // The thread or draft under the cursor.
    let at = diff.doc.annotation_at(cursor);
    let annotation = at
        .and_then(|i| diff.doc.annotations().get(i as usize))
        .cloned();
    let notice = |state: &mut State, n: Notice| {
        state.notice = Some(n);
        Some(Vec::new())
    };
    let in_range = diff.range.is_some();
    if in_range
        && matches!(
            action,
            Action::Comment | Action::Suggest | Action::FileComment | Action::SubmitReview
        )
    {
        let all = all_changes(state);
        return notice(
            state,
            Notice::Error(format!("Comments anchor to the whole PR: {all} to comment")),
        );
    }
    match action {
        Action::ToggleSinceReview => {
            if in_range {
                let all = all_changes(state);
                return notice(state, Notice::Error(format!("First {all}")));
            }
            let active = !diff.doc.since_active();
            let mut known = false;
            diff_screen::preserving_position(screen, &mut diff.doc, |doc| {
                known = doc.show_since(active);
            });
            if known {
                state.info(if active {
                    "Showing changes since your last review"
                } else {
                    "Showing all changes"
                });
                return Some(Vec::new());
            }
            diff.since_requested = true;
            if diff.last_review == LastReview::Unknown {
                let Some(login) = viewer else {
                    // Without a login, only the local record can say.
                    diff.last_review = LastReview::None;
                    return Some(start_since_review(state));
                };
                state.info("Looking up your last review…");
                return Some(vec![Cmd::Api(Api::FetchLastReview { pr, login })]);
            }
            Some(start_since_review(state))
        }
        Action::PickCommits => {
            if diff.commits.is_empty() {
                state.info("Listing commits…");
                return Some(vec![Cmd::Git(Git::ListCommits(pr))]);
            }
            Some(state.open_picker(picker::Kind::Commits { mark: None }))
        }
        Action::Back if screen.selection.is_some() => {
            screen.selection = None;
            Some(Vec::new())
        }
        // `c` on a thread replies to it.
        Action::Comment
            if let Some(Annotation {
                key: AnnotationKey::Thread(thread_id),
                can_reply,
                ..
            }) = &annotation =>
        {
            if !can_reply {
                return notice(state, Notice::Info("You can't reply to this one".into()));
            }
            let thread_id = thread_id.clone();
            Some(state.compose(ComposeTarget::Reply { thread_id }, ""))
        }
        Action::Comment => {
            let selection = screen.selection.take();
            match target(&diff.doc, cursor, selection) {
                Ok(target) => {
                    if let ComposeTarget::File {
                        reason: Some(reason),
                        ..
                    } = &target
                    {
                        state.info(reason.clone());
                    }
                    Some(state.compose(target, ""))
                }
                Err(err) => notice(state, Notice::Error(err)),
            }
        }
        Action::FileComment => {
            // On a draft: turn it into a file comment (e.g. after GitHub
            // rejected its line). Elsewhere: comment on the file.
            if let Some(ann) = &annotation
                && let AnnotationKey::Draft(id) = ann.key
            {
                if let Some(d) = diff.review.pending.iter_mut().find(|d| d.id == id) {
                    d.line = None;
                    d.start_line = None;
                    d.start_side = None;
                    d.error = None;
                }
                diff.refresh_annotations();
                let save = Cmd::SaveReview(pr, diff.review.clone());
                state.info("Now a comment on the file");
                return Some(vec![save]);
            }
            let path = diff.doc.files().get(cursor.file)?.meta.path().to_owned();
            Some(state.compose(ComposeTarget::File { path, reason: None }, ""))
        }
        Action::Suggest => {
            let selection = screen.selection.take();
            let target = match target(&diff.doc, cursor, selection) {
                Ok(target) => target,
                Err(err) => return notice(state, Notice::Error(err)),
            };
            let ComposeTarget::Line { path, start, end } = target else {
                return notice(
                    state,
                    Notice::Error("Suggestions need lines GitHub can comment on".into()),
                );
            };
            if start.side != Side::Right || end.side != Side::Right {
                return notice(
                    state,
                    Notice::Error("Suggestions apply to new lines (not deleted ones)".into()),
                );
            }
            let text = diff.doc.files().get(cursor.file)?.text()?;
            let original: Vec<String> = (start.line..=end.line)
                .map(|n| text.new.line_no(n).to_owned())
                .collect();
            Some(vec![Cmd::Edit {
                text: original.join("\n") + "\n",
                purpose: EditPurpose::Suggest {
                    path,
                    start,
                    end,
                    original,
                },
            }])
        }
        Action::ResolveThread => {
            let Some(ann) = annotation else {
                return notice(state, Notice::Info("Move to a thread to resolve it".into()));
            };
            let AnnotationKey::Thread(thread_id) = ann.key else {
                return notice(state, Notice::Info("Drafts can't be resolved".into()));
            };
            let resolved = !ann.resolved;
            if (resolved && !ann.can_resolve) || (!resolved && !ann.can_unresolve) {
                return notice(state, Notice::Info("You can't change this thread".into()));
            }
            // Optimistic; rolled back if GitHub refuses.
            if let Some(t) = diff.threads.iter_mut().find(|t| t.id == thread_id) {
                t.resolved = resolved;
            }
            diff.refresh_annotations();
            Some(vec![Cmd::Api(Api::SetResolved {
                pr,
                thread_id,
                resolved,
            })])
        }
        Action::DeleteDraft => {
            let Some(AnnotationKey::Draft(id)) = annotation.map(|a| a.key) else {
                return notice(
                    state,
                    Notice::Info("Move to a draft comment to delete it".into()),
                );
            };
            let at = diff.review.pending.iter().position(|d| d.id == id)?;
            diff.deleted = Some(diff.review.pending.remove(at));
            diff.refresh_annotations();
            let save = Cmd::SaveReview(pr, diff.review.clone());
            let undo = state.first_key(Action::UndoDelete);
            state.info(format!("Draft deleted · {undo} brings it back"));
            Some(vec![save])
        }
        Action::UndoDelete => {
            let Some(mut draft) = diff.deleted.take() else {
                return notice(state, Notice::Info("No deleted draft to bring back".into()));
            };
            if diff.review.pending.iter().any(|d| d.id == draft.id) {
                draft.id = diff.review.next_draft_id();
            }
            diff.review.pending.push(draft);
            diff.refresh_annotations();
            let save = Cmd::SaveReview(pr, diff.review.clone());
            state.info("Draft restored");
            Some(vec![save])
        }
        Action::Open => {
            if let AnnotationKey::Draft(id) = annotation?.key {
                let body = diff
                    .review
                    .pending
                    .iter()
                    .find(|d| d.id == id)?
                    .body
                    .clone();
                return Some(state.compose(ComposeTarget::Draft { id }, &body));
            }
            diff.doc.toggle_thread(at?);
            Some(Vec::new())
        }
        Action::SubmitReview => {
            if diff.refs.is_none() {
                return notice(state, Notice::Error("The diff hasn't loaded yet".into()));
            }
            state.overlay = Some(Overlay::Submit(Box::new(SubmitDialog::new(&state.theme))));
            Some(Vec::new())
        }
        _ => None,
    }
}

/// Compares with the head of your last review, once known.
#[must_use]
pub(crate) fn start_since_review(state: &mut State) -> Vec<Cmd> {
    let Some((screen, diff)) = state.diff_parts() else {
        return Vec::new();
    };
    let pr = screen.pr.clone();
    let old = match &diff.last_review {
        LastReview::At(oid) => Some(oid.to_string()),
        LastReview::Unknown | LastReview::None => diff.review.last_reviewed_head.clone(),
    };
    let head = diff.head();
    let problem = match (&old, &head) {
        (None, _) => Some("You haven't reviewed this PR yet"),
        (_, None) => Some("The diff hasn't loaded yet"),
        (Some(old), Some(head)) if **old == **head => {
            Some("Nothing new: you reviewed the current head")
        }
        _ => None,
    };
    if let Some(message) = problem {
        diff.since_requested = false;
        state.info(message);
        return Vec::new();
    }
    state.info("Comparing with your last review…");
    vec![Cmd::Git(Git::SinceReview {
        pr,
        old_head: old.unwrap_or_default(),
    })]
}

/// Shows what `choice` picks: the whole PR, the changes since your last
/// review, or a commit (from `mark` to it, if a range was marked).
#[must_use]
pub(crate) fn apply_commit_choice(
    state: &mut State,
    choice: PickItem,
    mark: Option<PickItem>,
) -> Vec<Cmd> {
    let width = state.size.0;
    let pr = match state.screen() {
        Screen::Diff(screen) => screen.pr.clone(),
        Screen::Page(_) => return Vec::new(),
    };
    let Some(base_ref) = state.base_ref(&pr) else {
        return Vec::new();
    };
    let Some((screen, diff)) = state.diff_parts() else {
        return Vec::new();
    };
    let since = choice == PickItem::SinceReview;
    let range = match choice {
        // Already on the whole PR: only "since your review" changes.
        PickItem::All | PickItem::SinceReview if diff.range.is_none() => {
            if diff.doc.since_active() == since {
                return Vec::new();
            }
            return review_action(state, Action::ToggleSinceReview).unwrap_or_default();
        }
        PickItem::All | PickItem::SinceReview => None,
        PickItem::Commit(i) => {
            let j = match mark {
                Some(PickItem::Commit(m)) => m,
                _ => i,
            };
            let (first, last) = (i.min(j), i.max(j));
            // The picker's indices, into the list it showed.
            let (Some(from), Some(to)) = (diff.commits.get(first), diff.commits.get(last)) else {
                return Vec::new();
            };
            let (from, to) = (&from.oid, &to.oid);
            let label = if first == last {
                short_sha(to).to_owned()
            } else {
                format!("{}..{}", short_sha(from), short_sha(to))
            };
            Some(diff_screen::RangeView {
                label,
                from: format!("{from}^"),
                to: to.to_string(),
            })
        }
    };
    let cmd_range = range.as_ref().map(|r| (r.from.clone(), r.to.clone()));
    diff.restart(range);
    // Since your review: compared once the whole PR is back.
    diff.since_requested = since;
    let job = diff.job;
    *screen = DiffScreen::new(pr.clone(), width);
    vec![Cmd::Git(Git::LoadDiff {
        pr,
        job,
        base_ref,
        range: cmd_range,
    })]
}

#[must_use]
pub(crate) fn on_compose_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Compose(compose)) = &mut state.overlay else {
        return Vec::new();
    };
    if compose.sending {
        return Vec::new();
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc if compose.text().trim().is_empty() || compose.confirm_discard => {
            state.overlay = None;
        }
        KeyCode::Esc => {
            compose.confirm_discard = true;
            compose.error = Some("Press esc again to discard this comment".into());
        }
        KeyCode::Char('e') if ctrl => {
            return vec![Cmd::Edit {
                purpose: EditPurpose::Compose,
                text: compose.text(),
            }];
        }
        KeyCode::Char('s') if ctrl => return save_compose(state),
        _ => {
            compose.confirm_discard = false;
            compose.error = None;
            compose.input.input(key);
        }
    }
    Vec::new()
}

/// `ctrl-s` in the composer: drafts join the pending review (and are saved);
/// replies post right away.
#[must_use]
pub(crate) fn save_compose(state: &mut State) -> Vec<Cmd> {
    let Some(Overlay::Compose(compose)) = &mut state.overlay else {
        return Vec::new();
    };
    let body = compose.text();
    if body.trim().is_empty() {
        compose.error = Some("Write something first".into());
        return Vec::new();
    }
    let target = compose.target.clone();
    if let ComposeTarget::Conversation {
        subject_id,
        refresh,
        ..
    } = target
    {
        compose.sending = true;
        return vec![Cmd::Api(Api::AddComment {
            subject_id,
            body,
            refresh,
        })];
    }
    let Some((screen, diff)) = state.diff_parts() else {
        return Vec::new();
    };
    let pr = screen.pr.clone();
    match target {
        ComposeTarget::Reply { thread_id } => {
            if let Some(Overlay::Compose(compose)) = &mut state.overlay {
                compose.sending = true;
            }
            vec![Cmd::Api(Api::Reply {
                pr,
                thread_id,
                body,
            })]
        }
        ComposeTarget::Draft { id } => {
            if let Some(d) = diff.review.pending.iter_mut().find(|d| d.id == id) {
                d.body = body;
                d.error = None;
            }
            diff.refresh_annotations();
            let save = Cmd::SaveReview(pr, diff.review.clone());
            state.overlay = None;
            vec![save]
        }
        ComposeTarget::Conversation { .. } => Vec::new(),
        target @ (ComposeTarget::Line { .. } | ComposeTarget::File { .. }) => {
            let head = diff.head().unwrap_or_default();
            let id = diff.review.next_draft_id();
            let Some(draft) = draft(&target, &body, id, &head) else {
                return Vec::new();
            };
            diff.review.pending.push(draft);
            diff.refresh_annotations();
            let save = Cmd::SaveReview(pr, diff.review.clone());
            let count = diff.review.pending.len();
            state.overlay = None;
            state.info(format!(
                "Added to your review ({count} pending). Submit with {}",
                state.first_key(Action::SubmitReview)
            ));
            vec![save]
        }
    }
}

#[must_use]
pub(crate) fn on_submit_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Submit(dialog)) = &mut state.overlay else {
        return Vec::new();
    };
    if dialog.sending {
        return Vec::new();
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => state.overlay = None,
        KeyCode::Tab => dialog.cycle(true),
        KeyCode::BackTab => dialog.cycle(false),
        KeyCode::Char('e') if ctrl => {
            return vec![Cmd::Edit {
                purpose: EditPurpose::Summary,
                text: dialog.input.lines().join("\n"),
            }];
        }
        KeyCode::Char('s') if ctrl => {
            let event = dialog.event;
            let body = dialog.input.lines().join("\n");
            if event == ReviewEvent::RequestChanges && body.trim().is_empty() {
                dialog.error = Some("Requesting changes needs a summary".into());
                return Vec::new();
            }
            dialog.sending = true;
            dialog.error = None;
            if let Some((screen, diff)) = state.diff_parts() {
                return vec![Cmd::Api(Api::SubmitReview {
                    pr: screen.pr.clone(),
                    head: diff.head().unwrap_or_default(),
                    drafts: diff.review.pending.clone(),
                    event,
                    body,
                })];
            }
        }
        _ => {
            dialog.error = None;
            dialog.input.input(key);
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures::thread;

    #[test]
    fn outdated_threads_move_when_mapped() {
        let threads = vec![
            thread("current", Some(3), false, false),
            thread("mapped", None, false, true),
            thread("lost", None, false, true),
        ];
        let mapped = HashMap::from([
            (NodeId::new("mapped"), Some(12)),
            (NodeId::new("lost"), None),
        ]);
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
        let d = draft(&range, "x", 1, "head").unwrap();
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
        let d = draft(&single, "x", 2, "head").unwrap();
        assert_eq!(
            (d.start_line, d.start_side),
            (None, None),
            "single lines have no start"
        );

        let file = ComposeTarget::File {
            path: "a.rs".into(),
            reason: None,
        };
        assert_eq!(draft(&file, "x", 3, "head").unwrap().line, None);
        assert!(
            draft(
                &ComposeTarget::Reply {
                    thread_id: NodeId::new("t")
                },
                "x",
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
