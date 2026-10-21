//! Reviewing: comment targets, drafts, suggestions, and turning GitHub
//! threads and local drafts into annotations.

use std::collections::{HashMap, HashSet};

use crate::api::model::{NewThread, ReviewEvent, ReviewThread, Side as ApiSide};
use crate::api::model::{NodeId, PendingReview, PrDetail, PrRef};
use crate::api::{ApiError, GitHub};
use crate::data::{DataKey, Picked};
use crate::diff::anchor::{AnchorError, LinePos, Side};
use crate::store::{DraftComment, DraftSide, ReviewState};
use crate::theme::{Bg, Theme};
use crate::ui::annotations::{Annotation, AnnotationComment, AnnotationKey, Place};
use crate::ui::diff_doc::Viewed;
use crate::ui::diff_doc::{Doc, Pos, Row};
use crate::ui::text::short_sha;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::TextArea;

use crate::api::change::Change;
use crate::compose::{Compose, Target, editor};
use crate::diff_screen::{self, DiffOf, DiffScreen, LastReview};
use crate::forms::Sent;
use crate::keymap::Action;
use crate::picker::{self, PickItem};
use crate::route::{Route, pulls};
use crate::state::{Api, Cmd, Git, OutdatedThread, Overlay, Screen, State};
use crate::ui::page::{Link, ReviewLink};
use crate::ui::pages::PrTab;
use crate::work::{Cx, Failure, Job};

/// A review's comment being written: where it goes, or what it answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Comment {
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
}

impl Comment {
    /// The composer's title.
    pub fn title(&self) -> String {
        let side = |s: Side| match s {
            Side::Left => "old",
            Side::Right => "new",
        };
        match self {
            Comment::Line { path, start, end } if start == end => {
                format!("Comment on {path}:{} ({})", end.line, side(end.side))
            }
            Comment::Line { path, start, end } => format!(
                "Comment on {path}, lines {}–{} ({})",
                start.line,
                end.line,
                side(end.side)
            ),
            Comment::File { path, .. } => format!("Comment on the file {path}"),
            Comment::Reply { .. } => "Reply".to_owned(),
            Comment::Draft { .. } => "Edit draft".to_owned(),
        }
    }

    /// What ctrl-s does, for the composer's hint.
    pub fn save(&self) -> &'static str {
        match self {
            Comment::Reply { .. } => "post reply",
            Comment::Draft { .. } => "save draft",
            Comment::Line { .. } | Comment::File { .. } => "add to review",
        }
    }

    /// Why a comment asked for on lines is on the whole file.
    pub fn note(&self) -> Option<&str> {
        match self {
            Comment::File {
                reason: Some(reason),
                ..
            } => Some(reason),
            _ => None,
        }
    }
}

/// A suggested change and what it replaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub start_line: u32,
    pub original: Vec<String>,
    pub suggested: Vec<String>,
}

/// The row of the diff the composer opens under: the last selected line,
/// the end of the thread or draft it's about, or the file's header.
pub fn compose_anchor(doc: &Doc, screen: &DiffScreen, target: &Target) -> Pos {
    let cursor = screen.cursor;
    match target {
        Target::Review(Comment::File { .. }) => Pos { row: 0, ..cursor },
        Target::Review(Comment::Reply { .. } | Comment::Draft { .. }) | Target::Edit(_) => {
            let ann = doc.annotation_under(cursor, screen.half);
            let end = doc.line_end(cursor);
            (end.row + 1..)
                .map(|row| Pos { row, ..cursor })
                .take_while(|&p| {
                    matches!(doc.row(p), Some(Row::Thread(_))) && doc.annotation_at(p) == ann
                })
                .last()
                .unwrap_or(end)
        }
        // Under all of the line, wrapped.
        _ => doc.line_end(
            screen
                .selection
                .filter(|s| doc.to_global(*s) > doc.to_global(cursor))
                .unwrap_or(cursor),
        ),
    }
}

pub struct SubmitDialog {
    pub pr: PrRef,
    pub event: ReviewEvent,
    pub input: TextArea<'static>,
    pub sent: Sent,
    /// Enter submits: opened to approve from the pull request's page.
    pub quick: bool,
    /// Your saved review (its drafts go with it), read from disk when the
    /// pull request's diff isn't open to hold it.
    pub saved: Option<ReviewState>,
    /// The key (esc or ctrl-c) pressed once with a summary typed: only that
    /// key, pressed again, throws the summary away.
    pub discard_armed: Option<&'static str>,
    /// The comments your pending review on GitHub holds, which go with
    /// it; `None` until GitHub says.
    pub on_github: Option<u64>,
}

impl SubmitDialog {
    pub fn new(theme: &Theme, pr: PrRef) -> Self {
        let mut input = editor(theme, "");
        input.set_placeholder_text("Summary (optional for comments and approvals)");
        input.set_placeholder_style(theme.meta(Bg::ContainerHigh));
        Self {
            pr,
            event: ReviewEvent::Comment,
            input,
            sent: Sent::default(),
            quick: false,
            saved: None,
            discard_armed: None,
            on_github: None,
        }
    }

    /// Your pending drafts, wherever they're held: `None` while they're
    /// still being read.
    pub fn drafts<'a>(&'a self, state: &'a State) -> Option<&'a [DraftComment]> {
        match state.diffs.get(&DiffOf::Pr(self.pr.clone())) {
            Some(diff) => Some(&diff.inputs().review.pending),
            None => self.saved.as_ref().map(|s| s.pending.as_slice()),
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
    /// The comments your pending review on GitHub held after, if known.
    pub on_github: Option<u64>,
    /// GitHub couldn't be reached, and nothing was sent: the review, to
    /// queue ([`crate::offline`]).
    pub unsent: Option<Box<crate::offline::queue::Review>>,
}

impl SubmitOutcome {
    /// `drafts` without those GitHub took, each refused one saying why.
    fn keep_refused(&self, drafts: &mut Vec<DraftComment>) {
        drafts.retain(|d| !self.accepted.contains(&d.id));
        for draft in drafts {
            draft.error = (self.rejected.iter())
                .find(|(id, _)| *id == draft.id)
                .map(|(_, reason)| reason.clone());
        }
    }
}

pub fn suggestion_body(suggested: &str) -> String {
    format!("```suggestion\n{}\n```", suggested.trim_end_matches('\n'))
}

/// Where a new comment goes, from the cursor or a visual selection (on
/// `half` of split rows). Lines GitHub won't accept become a file comment
/// (with the reason); a range that crosses GitHub's hunks is refused.
pub fn target(
    doc: &Doc,
    cursor: Pos,
    half: Side,
    selection: Option<Pos>,
) -> Result<Comment, String> {
    let file = doc.files().get(cursor.file).ok_or("No file here")?;
    let path = file.meta.path().to_owned();
    if selection.is_none() && doc.row(cursor) == Some(Row::Spacer) {
        return Err("This is between files: move into one to comment".into());
    }
    let from = match selection {
        Some(anchor) if anchor.file == cursor.file => anchor,
        Some(_) => return Err("A selection can't span files".into()),
        None => cursor,
    };
    let Some((start, end)) = doc.line_range(from, cursor, half) else {
        return Ok(Comment::File { path, reason: None });
    };
    let Some(commentable) = doc.commentable(cursor.file) else {
        return Err("This file's diff hasn't loaded yet".into());
    };
    match commentable.validate(start, end) {
        Ok(()) => Ok(Comment::Line { path, start, end }),
        Err(AnchorError::NotCommentable) => Ok(Comment::File {
            path,
            reason: Some(
                "That line isn't part of GitHub's diff, so this will be a comment on the file."
                    .into(),
            ),
        }),
        Err(err) => Err(format!("Can't comment there: {err}")),
    }
}

/// Where a draft is: its lines, or its file.
fn place(d: &DraftComment) -> Comment {
    let path = d.path.clone();
    let Some(line) = d.line else {
        return Comment::File { path, reason: None };
    };
    let end = LinePos {
        side: side_of(d.side),
        line,
    };
    let start = LinePos {
        side: side_of(d.start_side.unwrap_or(d.side)),
        line: d.start_line.unwrap_or(line),
    };
    Comment::Line { path, start, end }
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
pub fn draft(target: &Comment, body: &str, id: u64, commit: &str) -> Option<DraftComment> {
    let base = |path: &str| DraftComment {
        id,
        path: path.to_owned(),
        body: body.to_owned(),
        side: DraftSide::Right,
        commit: commit.to_owned(),
        ..Default::default()
    };
    match target {
        Comment::Line { path, start, end } => {
            let ranged = start != end;
            Some(DraftComment {
                side: draft_side(end.side),
                line: Some(end.line),
                start_line: ranged.then_some(start.line),
                start_side: ranged.then_some(draft_side(start.side)),
                ..base(path)
            })
        }
        Comment::File { path, .. } => Some(base(path)),
        Comment::Reply { .. } | Comment::Draft { .. } => None,
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
/// Threads and comments in `fresh` came in since you started looking.
pub fn annotations(
    threads: &[ReviewThread],
    mapped: &HashMap<NodeId, Option<u32>>,
    drafts: &[DraftComment],
    fresh: &HashSet<NodeId>,
) -> Vec<Annotation> {
    let mut out: Vec<Annotation> = threads
        .iter()
        .map(|t| {
            let side = match t.side {
                ApiSide::Left => Side::Left,
                ApiSide::Right => Side::Right,
            };
            let moved_to = mapped.get(&t.id).copied().flatten();
            let place = match (t.file_level, t.outdated, t.line, moved_to) {
                (true, outdated, ..) => Place::File { outdated },
                (_, false, Some(line), _) => Place::Line {
                    at: LinePos { side, line },
                    start: t.start_line,
                },
                (.., Some(line)) => Place::Moved(LinePos { side, line }),
                _ => Place::Outdated(t.original_line),
            };
            Annotation {
                key: AnnotationKey::Thread(t.id.clone()),
                path: t.path.clone(),
                place,
                resolved: t.resolved,
                comments: t
                    .comments
                    .iter()
                    .map(|c| AnnotationComment {
                        author: c.author.clone(),
                        body: c.body.clone(),
                        created_at: c.created_at.clone(),
                        pending: c.pending,
                        new: fresh.contains(&c.id),
                        editable: Some(c.editable.clone()),
                    })
                    .collect(),
                left_out: t.comments.left_out(),
                error: None,
                can_reply: t.can_reply,
                can_resolve: t.can_resolve,
                can_unresolve: t.can_unresolve,
                new: fresh.contains(&t.id),
            }
        })
        .collect();
    out.extend(drafts.iter().map(|d| Annotation {
        key: AnnotationKey::Draft(d.id),
        path: d.path.clone(),
        place: match d.line {
            Some(line) => Place::Line {
                at: LinePos {
                    side: side_of(d.side),
                    line,
                },
                start: d.start_line,
            },
            None => Place::File { outdated: false },
        },
        resolved: false,
        comments: vec![AnnotationComment {
            author: "You".into(),
            body: d.body.clone(),
            created_at: String::new(),
            pending: false,
            new: false,
            editable: None,
        }],
        left_out: 0,
        error: d.error.clone(),
        can_reply: false,
        can_resolve: false,
        can_unresolve: false,
        new: false,
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
    // GitHub couldn't be reached: the review and its drafts move to the
    // queue, to be sent when it's back.
    if let Some(review) = &outcome.unsent {
        let moved = SubmitOutcome {
            accepted: review.drafts.iter().map(|d| d.id).collect(),
            ..SubmitOutcome::default()
        };
        let mut cmds = on_submitted(state, pr, &moved);
        if matches!(state.overlay, Some(Overlay::Submit(_))) {
            state.overlay = None;
        }
        cmds.push(crate::offline::queue_review(state, review));
        return cmds;
    }
    let apply = |review: &mut ReviewState, head: Option<String>| {
        outcome.keep_refused(&mut review.pending);
        if outcome.submitted {
            review.last_reviewed_head = head;
        }
    };
    let mut cmds = vec![Cmd::Api(Api::FetchThreads(pr.clone()))];
    if let Some(diff) = state.diffs.get_mut(&DiffOf::Pr(pr.clone())) {
        let head = diff.head().map(|h| h.to_string());
        cmds.extend(diff.edit_review(pr, |i| apply(&mut i.review, head)));
    } else if let Some(Overlay::Submit(dialog)) = &mut state.overlay
        && let Some(saved) = &mut dialog.saved
    {
        // Without the diff, the dialog holds what's saved: save it back.
        let key = DataKey::Pr(pr.clone());
        let detail = (state.data.get(&key)).and_then(|r| r.data.as_ref().and_then(PrDetail::pick));
        let head = detail.map(|d| d.head_oid.clone());
        apply(saved, head);
        cmds.push(Cmd::SaveReview(pr.clone(), saved.clone()));
    }
    if outcome.submitted {
        let approved = matches!(
            &state.overlay,
            Some(Overlay::Submit(d)) if d.event == ReviewEvent::Approve
        );
        state.overlay = None;
        state.info(if approved {
            format!("Approved {pr}")
        } else {
            "Review submitted".to_owned()
        });
        cmds.extend(state.load_visible(false));
    } else if let Some(Overlay::Submit(dialog)) = &mut state.overlay {
        dialog.sent.sending = false;
        dialog.on_github = outcome.on_github.or(dialog.on_github);
        dialog.sent.error = Some(match (&outcome.error, outcome.rejected.len()) {
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
    result: Result<String, crate::work::Failure>,
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
                dialog.input = editor(theme, text.trim_end());
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
                return state.just_say("No change suggested");
            }
            let body = suggestion_body(&suggested.join("\n"));
            let target = Target::Review(Comment::Line { path, start, end });
            let mut compose = Compose::new(theme, target, &body);
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

/// Review actions on the diff screen: commenting, threads, drafts and
/// submitting, and which changes are shown.
#[must_use]
pub(crate) fn review_action(state: &mut State, action: Action) -> Vec<Cmd> {
    let viewer = state.viewer.clone();
    let Some((screen, diff)) = state.diff_parts() else {
        return state.just_say("The diff hasn't loaded yet");
    };
    let Some(pr) = screen.of.pr().cloned() else {
        return state.just_say(format!("{}: on pull requests", action.description()));
    };
    let (cursor, half) = (screen.cursor, screen.half);
    // The thread or draft under the cursor.
    let at = diff.doc.annotation_under(cursor, half);
    let annotation = at
        .and_then(|i| diff.doc.annotations().get(i as usize))
        .cloned();
    let error = |state: &mut State, text: &str| {
        state.error(text);
        Vec::new()
    };
    let in_range = diff.range().is_some();
    if in_range
        && matches!(
            action,
            Action::Comment | Action::Suggest | Action::FileComment | Action::SubmitReview
        )
    {
        let all = all_changes(state);
        return error(
            state,
            &format!("Comments anchor to the whole PR: {all} to comment"),
        );
    }
    match action {
        Action::ToggleSinceReview => {
            if in_range {
                let all = all_changes(state);
                return error(state, &format!("First {all}"));
            }
            // Another review's comparison isn't yours: yours is made anew.
            if diff.since_commit.take().is_some() {
                diff.doc.set_since(None, false);
            }
            // A push since the diff was made: the diff starts again at
            // it, and compares once it's in.
            let head = diff.head().map(|h| h.to_string());
            if crate::live::pushed_past(state, &pr, head.as_deref())
                && let Some(base) = state.pr_base(&pr)
                && let Some((screen, diff)) = state.diff_parts()
            {
                diff.restart(None);
                diff.since_requested = true;
                screen.restart();
                let job = diff.job;
                state.info("Loading the new push, to compare with your last review…");
                return vec![Cmd::Git(Git::LoadDiff {
                    of: pr.into(),
                    job,
                    base,
                    range: None,
                })];
            }
            let Some((screen, diff)) = state.diff_parts() else {
                return Vec::new();
            };
            let active = !diff.doc.since_active();
            if diff_screen::preserving_position(screen, diff, |d| d.doc.show_since(active)) {
                return state.just_say(if active {
                    "Showing changes since your last review"
                } else {
                    "Showing all changes"
                });
            }
            diff.since_requested = true;
            match (&diff.inputs().last_review, viewer) {
                // GitHub hasn't answered, or couldn't: ask it (again), and
                // it isn't known until it answers.
                (LastReview::Unknown | LastReview::Failed(_), Some(login)) => {
                    diff.edit(|i| i.last_review = LastReview::Unknown);
                    state.info("Looking up your last review…");
                    vec![Cmd::Api(Api::FetchLastReview { pr, login })]
                }
                // Without a login, only the local record can say.
                (LastReview::Unknown | LastReview::Failed(_), None) => {
                    diff.edit(|i| i.last_review = LastReview::None);
                    Vec::new()
                }
                (LastReview::None | LastReview::At(_), _) => {
                    if !diff.listed() {
                        state.info("Comparing once the diff has loaded");
                    }
                    Vec::new()
                }
            }
        }
        Action::PickCommits => {
            if diff.inputs().commits.is_empty() {
                state.info("Listing commits…");
                return vec![Cmd::Git(Git::ListCommits(pr))];
            }
            state.open_picker(picker::Kind::Commits { mark: None })
        }
        // `c` on a thread replies to it; with lines selected, it comments
        // on them, wherever the cursor has gone.
        Action::Comment
            if screen.selection.is_none()
                && let Some(Annotation {
                    key: AnnotationKey::Thread(thread_id),
                    can_reply,
                    ..
                }) = &annotation =>
        {
            if !can_reply {
                return state.just_say("You can't reply to this one");
            }
            let thread_id = thread_id.clone();
            state.compose(Target::Review(Comment::Reply { thread_id }), "")
        }
        // `c` on a draft's rows comments where it is, as on its line.
        Action::Comment
            if screen.selection.is_none()
                && matches!(diff.doc.row(cursor), Some(Row::Thread(_)))
                && let Some(&AnnotationKey::Draft(id)) = annotation.as_ref().map(|a| &a.key) =>
        {
            let pending = &diff.inputs().review.pending;
            let Some(target) = pending.iter().find(|d| d.id == id).map(place) else {
                return Vec::new();
            };
            state.compose(Target::Review(target), "")
        }
        Action::Comment => {
            // The selection stays until the comment is added.
            match target(&diff.doc, cursor, half, screen.selection) {
                Ok(target) => {
                    if let Some(reason) = target.note() {
                        state.info(reason);
                    }
                    state.compose(Target::Review(target), "")
                }
                Err(err) => error(state, &err),
            }
        }
        Action::FileComment => {
            // On a draft: turn it into a file comment (e.g. after GitHub
            // rejected its line). Elsewhere: comment on the file.
            if let Some(ann) = &annotation
                && let AnnotationKey::Draft(id) = ann.key
            {
                let save = diff.edit_review(&pr, |i| {
                    if let Some(d) = i.review.pending.iter_mut().find(|d| d.id == id) {
                        d.line = None;
                        d.start_line = None;
                        d.start_side = None;
                        d.error = None;
                    }
                });
                state.info("Now a comment on the file");
                return save;
            }
            let Some(file) = diff.doc.files().get(cursor.file) else {
                return Vec::new();
            };
            let path = file.meta.path().to_owned();
            state.compose(Target::Review(Comment::File { path, reason: None }), "")
        }
        Action::Suggest => {
            let target = match target(&diff.doc, cursor, half, screen.selection) {
                Ok(target) => target,
                Err(err) => return error(state, &err),
            };
            let Comment::Line { path, start, end } = target else {
                return error(state, "Suggestions need lines GitHub can comment on");
            };
            if start.side != Side::Right || end.side != Side::Right {
                return error(state, "Suggestions apply to new lines (not deleted ones)");
            }
            let Some(text) = diff.doc.files().get(cursor.file).and_then(|f| f.text()) else {
                return error(state, "Suggestions need the file's text");
            };
            let original: Vec<String> = (start.line..=end.line)
                .map(|line| text.line(LinePos { line, ..start }).to_owned())
                .collect();
            vec![Cmd::Edit {
                text: original.join("\n") + "\n",
                purpose: EditPurpose::Suggest {
                    path,
                    start,
                    end,
                    original,
                },
            }]
        }
        Action::ResolveThread => {
            let Some(ann) = annotation else {
                return state.just_say("Move to a thread to resolve it");
            };
            let AnnotationKey::Thread(thread_id) = ann.key else {
                return state.just_say("Drafts can't be resolved");
            };
            let resolved = !ann.resolved;
            if (resolved && !ann.can_resolve) || (!resolved && !ann.can_unresolve) {
                return state.just_say("You can't change this thread");
            }
            // Optimistic; rolled back if GitHub refuses. The cursor stays
            // on the thread as it folds or unfolds.
            diff_screen::preserving_position(screen, diff, |d| {
                d.edit(|i| i.set_resolved(&thread_id, resolved));
            });
            crate::work::start(Resolve {
                pr,
                thread: thread_id,
                resolved,
            })
        }
        Action::Delete => match annotation.map(|a| a.key) {
            Some(AnnotationKey::Draft(id)) => delete_draft(state, &pr, id),
            _ => state.just_say("Move to a draft comment to delete it"),
        },
        Action::UndoDelete => restore_draft(state, &pr),
        // On a thread or draft (the diff handles the rest).
        Action::Open => {
            if let Some(&AnnotationKey::Draft(id)) = annotation.as_ref().map(|a| &a.key) {
                return edit_draft(state, &pr, id);
            }
            // A line of a comment with a link follows it.
            let url = match (diff.doc.row(cursor), diff.doc.files().get(cursor.file)) {
                (Some(Row::Thread(t)), Some(file)) => file
                    .thread_row(t)
                    .and_then(|r| r.line.links.first().cloned()),
                _ => None,
            };
            if let Some(url) = url {
                return state.follow(&crate::ui::page::Link::Url(url));
            }
            if let Some(at) = at {
                diff.doc.toggle_thread(at);
            }
            Vec::new()
        }
        Action::SubmitReview => {
            if diff.refs.is_none() {
                return error(state, "The diff hasn't loaded yet");
            }
            state.overlay = Some(Overlay::Submit(Box::new(SubmitDialog::new(
                &state.theme,
                pr.clone(),
            ))));
            vec![Cmd::Api(Api::FetchPendingReview(pr))]
        }
        _ => state.just_say(action.not_here()),
    }
}

/// Opens the composer on your draft `id`. From the Review tab, it opens
/// Files changed on the draft first, where the composer goes.
#[must_use]
fn edit_draft(state: &mut State, pr: &PrRef, id: u64) -> Vec<Cmd> {
    let mut cmds = match state.screen() {
        Screen::Page(p) => match p.selected_link().cloned() {
            Some(link) => state.follow(&link),
            None => Vec::new(),
        },
        Screen::Diff(_) => Vec::new(),
    };
    let of = DiffOf::Pr(pr.clone());
    let pending = state.diffs.get(&of).map(|d| &d.inputs().review.pending);
    let Some(body) = pending
        .and_then(|p| p.iter().find(|d| d.id == id))
        .map(|d| d.body.clone())
    else {
        return cmds;
    };
    cmds.extend(state.compose(Target::Review(Comment::Draft { id }), &body));
    cmds
}

/// Deletes your draft `id`, which `ctrl-z` brings back.
#[must_use]
fn delete_draft(state: &mut State, pr: &PrRef, id: u64) -> Vec<Cmd> {
    let Some(diff) = state.diffs.get_mut(&DiffOf::Pr(pr.clone())) else {
        return Vec::new();
    };
    let Some(at) = diff.inputs().review.pending.iter().position(|d| d.id == id) else {
        return Vec::new();
    };
    let save = diff.edit_review(pr, |i| i.deleted = Some(i.review.pending.remove(at)));
    let undo = state.first_key(Action::UndoDelete);
    state.info(format!("Draft deleted · {undo} brings it back"));
    save
}

/// Brings back the draft just deleted.
#[must_use]
fn restore_draft(state: &mut State, pr: &PrRef) -> Vec<Cmd> {
    let Some(diff) = state.diffs.get_mut(&DiffOf::Pr(pr.clone())) else {
        return Vec::new();
    };
    let Some(mut draft) = diff.edit(|i| i.deleted.take()) else {
        return state.just_say("No deleted draft to bring back");
    };
    let save = diff.edit_review(pr, |i| {
        if i.review.pending.iter().any(|d| d.id == draft.id) {
            draft.id = i.review.next_draft_id();
        }
        i.review.pending.push(draft);
    });
    state.info("Draft restored");
    save
}

/// Your draft that's selected: under the diff's cursor, or a row of the
/// Review tab.
fn draft_here(state: &State) -> Option<(PrRef, u64)> {
    match state.screen() {
        Screen::Diff(screen) if screen.focus == diff_screen::Pane::Diff => {
            let doc = &state.diffs.get(&screen.of)?.doc;
            let ann = doc.annotation_under(screen.cursor, screen.half)?;
            match doc.annotations().get(ann as usize)?.key {
                AnnotationKey::Draft(id) => Some((screen.of.pr()?.clone(), id)),
                AnnotationKey::Thread(_) | AnnotationKey::Check(_) => None,
            }
        }
        Screen::Diff(_) => None,
        Screen::Page(p) => match p.selected_link()? {
            Link::Review(ReviewLink::Draft { pr, id, .. }) => Some((pr.clone(), *id)),
            _ => None,
        },
    }
}

/// The pull request whose Review tab is on screen.
fn review_tab(state: &State) -> Option<&PrRef> {
    match state.route()? {
        Route::Pulls(pulls::Pr {
            pr,
            tab: PrTab::Review,
        }) => Some(pr),
        _ => None,
    }
}

/// Whether your drafts' keys take `action` here: `,e` and `,D` on a
/// draft, as Enter and del do in Files changed, and on the Review tab del
/// on a draft and `ctrl-z` after deleting one.
pub fn takes(state: &State, action: Action) -> bool {
    match action {
        Action::EditComment | Action::DeleteComment => draft_here(state).is_some(),
        Action::Delete => review_tab(state).is_some() && draft_here(state).is_some(),
        Action::UndoDelete => review_tab(state).is_some(),
        _ => false,
    }
}

#[must_use]
fn draft_action(state: &mut State, action: Action) -> Vec<Cmd> {
    if action == Action::UndoDelete {
        return match review_tab(state).cloned() {
            Some(pr) => restore_draft(state, &pr),
            None => Vec::new(),
        };
    }
    let Some((pr, id)) = draft_here(state) else {
        return Vec::new();
    };
    match action {
        Action::EditComment => edit_draft(state, &pr, id),
        _ => delete_draft(state, &pr, id),
    }
}

/// Your drafts' rows in the menu, where one is selected.
fn offers(state: &State, out: &mut Vec<crate::nav::Doable>) {
    if draft_here(state).is_some() {
        for (action, label) in [
            (Action::EditComment, "Edit your draft"),
            (Action::DeleteComment, "Delete your draft"),
        ] {
            out.push(crate::nav::row("Edit", action, label, "", None));
        }
    }
}

/// Your drafts, edited and deleted where they're selected.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    takes,
    act: draft_action,
    offers,
    ..crate::features::Feature::NONE
};

/// Shows what `choice` picks: the whole PR, the changes since your last
/// review, or a commit (from `mark` to it, if a range was marked).
#[must_use]
pub(crate) fn apply_commit_choice(
    state: &mut State,
    choice: PickItem,
    mark: Option<usize>,
) -> Vec<Cmd> {
    let pr = match state.screen() {
        Screen::Diff(screen) => match screen.of.pr() {
            Some(pr) => pr.clone(),
            None => return Vec::new(),
        },
        Screen::Page(_) => return Vec::new(),
    };
    let Some(base) = state.pr_base(&pr) else {
        return Vec::new();
    };
    let Some((screen, diff)) = state.diff_parts() else {
        return Vec::new();
    };
    let since = choice == PickItem::SinceReview;
    let range = match choice {
        // Already on the whole PR: only "since your review" changes.
        PickItem::All | PickItem::SinceReview if diff.range().is_none() => {
            if diff.doc.since_active() == since {
                return Vec::new();
            }
            return review_action(state, Action::ToggleSinceReview);
        }
        PickItem::All | PickItem::SinceReview => None,
        PickItem::Commit(i) => {
            let j = mark.unwrap_or(i);
            let (first, last) = (i.min(j), i.max(j));
            // The picker's indices, into the list it showed.
            let commits = &diff.inputs().commits;
            let (Some(from), Some(to)) = (commits.get(first), commits.get(last)) else {
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
    screen.restart();
    vec![Cmd::Git(Git::LoadDiff {
        of: pr.into(),
        job,
        base,
        range: cmd_range,
    })]
}

/// `ctrl-s` in the composer on a review's comment: drafts join the
/// pending review (and are saved); replies post right away.
#[must_use]
pub(crate) fn save(state: &mut State, comment: Comment, body: String) -> Vec<Cmd> {
    let Some((screen, diff)) = state.diff_parts() else {
        return Vec::new();
    };
    let Some(pr) = screen.of.pr().cloned() else {
        return Vec::new();
    };
    match comment {
        Comment::Reply { thread_id } => {
            crate::forms::sending(state);
            let reply = Change::Reply {
                thread: thread_id,
                body,
            };
            vec![crate::act::send(reply, crate::act::By::Reply(pr))]
        }
        Comment::Draft { id } => {
            let save = diff.edit_review(&pr, |i| {
                if let Some(d) = i.review.pending.iter_mut().find(|d| d.id == id) {
                    d.body = body;
                    d.error = None;
                }
            });
            state.overlay = None;
            save
        }
        target @ (Comment::Line { .. } | Comment::File { .. }) => {
            let Some(head) = diff.head() else {
                return Vec::new();
            };
            let id = diff.inputs().review.next_draft_id();
            let Some(draft) = draft(&target, &body, id, &head) else {
                return Vec::new();
            };
            let save = diff.edit_review(&pr, |i| i.review.pending.push(draft));
            let count = diff.inputs().review.pending.len();
            screen.selection = None;
            state.overlay = None;
            state.info(format!(
                "Added to your review ({count} pending). Submit with {}",
                state.first_key(Action::SubmitReview)
            ));
            save
        }
    }
}

#[must_use]
pub(crate) fn on_submit_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Submit(dialog)) = &mut state.overlay else {
        return Vec::new();
    };
    if dialog.sent.sending {
        return Vec::new();
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            if crate::compose::may_discard(state, "esc", "discard") {
                state.overlay = None;
            }
        }
        KeyCode::Tab => dialog.cycle(true),
        KeyCode::BackTab => dialog.cycle(false),
        KeyCode::Char('e') if ctrl => {
            return vec![Cmd::Edit {
                purpose: EditPurpose::Summary,
                text: dialog.input.lines().join("\n"),
            }];
        }
        KeyCode::Char('s') if ctrl => return submit(state),
        KeyCode::Enter if dialog.quick => return submit(state),
        _ => {
            dialog.discard_armed = None;
            dialog.sent.error = None;
            dialog.input.input(key);
        }
    }
    Vec::new()
}

/// Submits the review in the dialog, with your drafts, on the head they
/// were written against: the diff's, or the pull request's from its page.
#[must_use]
fn submit(state: &mut State) -> Vec<Cmd> {
    let Some(Overlay::Submit(dialog)) = &state.overlay else {
        return Vec::new();
    };
    let (event, body, pr, seen) = (
        dialog.event,
        dialog.input.lines().join("\n"),
        dialog.pr.clone(),
        dialog.on_github.unwrap_or(0),
    );
    let drafts = dialog.drafts(state).map(<[DraftComment]>::to_vec);
    let head = match state.diffs.get(&DiffOf::Pr(pr.clone())) {
        Some(diff) => diff.head(),
        None => (state.pr(&pr)).and_then(|d| crate::git::Oid::parse(&d.head_oid).ok()),
    };
    let problem = match (&drafts, &head) {
        _ if event == ReviewEvent::RequestChanges && body.trim().is_empty() => {
            Some("Requesting changes needs a summary")
        }
        (None, _) => Some("Still reading your saved drafts; try again in a moment"),
        (_, None) => Some("The pull request hasn't loaded yet"),
        _ => None,
    };
    let Some(Overlay::Submit(dialog)) = &mut state.overlay else {
        return Vec::new();
    };
    let (None, Some(drafts), Some(head)) = (problem, drafts, head) else {
        dialog.sent.error = problem.map(Into::into);
        return Vec::new();
    };
    dialog.sent.sending = true;
    dialog.sent.error = None;
    crate::work::start(Submit {
        pr,
        head,
        drafts,
        event,
        body,
        seen,
    })
}

/// Opens the review dialog set to approve, from anywhere `pr` is on
/// screen. Your saved drafts go with it; without its diff open, they're
/// read from disk first.
#[must_use]
pub(crate) fn open_approve(state: &mut State, pr: PrRef) -> Vec<Cmd> {
    let mut dialog = SubmitDialog::new(&state.theme, pr.clone());
    dialog.event = ReviewEvent::Approve;
    dialog.quick = true;
    state.overlay = Some(Overlay::Submit(Box::new(dialog)));
    let mut cmds = vec![Cmd::Api(Api::FetchPendingReview(pr.clone()))];
    if !state.diffs.contains_key(&DiffOf::Pr(pr.clone())) {
        cmds.push(Cmd::LoadReview(pr));
    }
    cmds
}

/// Marks a file viewed on GitHub, or not: shown at once, and put back
/// as it was if GitHub refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetViewed {
    pub pr: PrRef,
    pub pull_request_id: NodeId,
    pub path: String,
    pub viewed: bool,
    pub previous: Viewed,
}

impl Job for SetViewed {
    type Out = ();
    /// Only the diff shows it, and it already does.
    const WRITES: bool = false;

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        let id = &self.pull_request_id;
        Ok(cx.gh.set_viewed(id, &self.path, self.viewed).await?)
    }

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        if let Err(err) = result {
            tracing::warn!(pr = %self.pr, %err, "viewed state update failed");
            let of = DiffOf::Pr(self.pr);
            diff_screen::edit(state, &of, |i| i.set_viewed(self.path, self.previous));
            state.error(format!("GitHub didn't save “viewed”: {err}"));
        }
        Vec::new()
    }
}

/// Resolves a review thread on GitHub, or unresolves it: shown at once,
/// and put back if GitHub refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolve {
    pub pr: PrRef,
    pub thread: NodeId,
    pub resolved: bool,
}

impl Job for Resolve {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        Ok(cx.gh.set_resolved(&self.thread, self.resolved).await?)
    }

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Vec<Cmd> {
        if let Err(err) = result {
            let of = DiffOf::Pr(self.pr);
            diff_screen::edit(state, &of, |i| i.set_resolved(&self.thread, !self.resolved));
            state.error(format!("GitHub didn't save that: {err}"));
        }
        Vec::new()
    }
}

/// Submits a review ([`submit_review`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submit {
    pub pr: PrRef,
    pub head: crate::git::Oid,
    pub drafts: Vec<DraftComment>,
    pub event: ReviewEvent,
    pub body: String,
    /// The comments in your pending review on GitHub you were shown: if
    /// it holds more, it isn't submitted.
    pub seen: u64,
}

impl Job for Submit {
    type Out = SubmitOutcome;

    /// Drafts GitHub accepted wait in a pending review there.
    fn wrote(result: &Result<SubmitOutcome, Failure>) -> bool {
        matches!(result, Ok(o) if o.error.is_none() || !o.accepted.is_empty())
    }

    async fn run(&self, cx: &Cx) -> Result<SubmitOutcome, Failure> {
        let review = Review {
            drafts: self.drafts.clone(),
            event: self.event,
            body: &self.body,
            seen: self.seen,
        };
        Ok(submit_review(&cx.gh, &self.pr, &self.head, review).await)
    }

    fn done(self, state: &mut State, result: Result<SubmitOutcome, Failure>) -> Vec<Cmd> {
        let outcome = result.unwrap_or_else(|err| SubmitOutcome {
            error: Some(format!("Couldn't submit: {err}")),
            ..SubmitOutcome::default()
        });
        on_submitted(state, &self.pr, &outcome)
    }
}

fn api_message(err: &ApiError) -> String {
    match err {
        ApiError::GraphQl(messages) => messages.join("; "),
        err => err.to_string(),
    }
}

/// A review to submit.
struct Review<'a> {
    drafts: Vec<DraftComment>,
    event: ReviewEvent,
    body: &'a str,
    /// The comments in your pending review on GitHub you were shown.
    seen: u64,
}

/// Submits a review: reuses the viewer's pending review on GitHub (or
/// starts one on `head`), adds each draft as a thread, and submits only if
/// GitHub accepted every draft. Each draft's fate is reported, so rejected
/// ones keep their text. A pending review holding comments you weren't
/// shown isn't submitted: they'd be published unseen.
async fn submit_review(gh: &GitHub, pr: &PrRef, head: &str, review: Review<'_>) -> SubmitOutcome {
    let mut outcome = SubmitOutcome::default();
    // GitHub can't be reached, and nothing was sent: to queue.
    let unsent = |review: &Review<'_>| {
        Some(Box::new(crate::offline::queue::Review {
            pr: pr.clone(),
            head: head.to_owned(),
            drafts: review.drafts.clone(),
            event: review.event,
            body: review.body.to_owned(),
            seen: review.seen,
        }))
    };
    // Offline, the review is kept to send later.
    let failed = |mut outcome: SubmitOutcome, err: &ApiError, what: &str| {
        if let ApiError::Offline(_) = err {
            outcome.unsent = unsent(&review);
        }
        outcome.error = Some(format!("{what}: {}", api_message(err)));
        outcome
    };
    let (review_id, held) = match gh.pending_review(pr).await {
        Ok((_, Some(pending))) if pending.comments > review.seen => {
            outcome.on_github = Some(pending.comments);
            outcome.error = Some(
                "Not sent: your pending review on GitHub holds comments you hadn't seen".into(),
            );
            return outcome;
        }
        Ok((_, Some(pending))) => match reusable(&pending, head) {
            Ok(()) => (pending.id, pending.comments),
            Err(message) => {
                outcome.error = Some(message);
                return outcome;
            }
        },
        Ok((pr_id, None)) => match gh.start_review(&pr_id, head).await {
            Ok(id) => (id, 0),
            Err(err) => return failed(outcome, &err, "Couldn't start the review"),
        },
        Err(err) => return failed(outcome, &err, "Couldn't reach the PR"),
    };
    for draft in review.drafts {
        match gh.add_review_thread(&review_id, &new_thread(&draft)).await {
            Ok(_) => outcome.accepted.push(draft.id),
            Err(err) => outcome.rejected.push((draft.id, api_message(&err))),
        }
    }
    // Those accepted are in it now, and were yours to see.
    outcome.on_github = Some(held + outcome.accepted.len() as u64);
    if !outcome.rejected.is_empty() {
        return outcome;
    }
    match gh
        .submit_review(&review_id, review.event, review.body)
        .await
    {
        Ok(()) => outcome.submitted = true,
        Err(err) => outcome.error = Some(format!("Couldn't submit: {}", api_message(&err))),
    }
    outcome
}

/// Submits a queued review ([`crate::offline::queue`]). What GitHub
/// refuses stays queued as refused, without the comments it took (they
/// wait in your pending review there, counted as seen).
pub(crate) async fn send_review(
    gh: &GitHub,
    queued: crate::offline::queue::Review,
) -> crate::offline::queue::Sent {
    use crate::offline::queue::Sent;
    let review = Review {
        drafts: queued.drafts.clone(),
        event: queued.event,
        body: &queued.body,
        seen: queued.seen,
    };
    let outcome = submit_review(gh, &queued.pr, &queued.head, review).await;
    if outcome.unsent.is_some() {
        return Sent::Unsent;
    }
    if outcome.submitted {
        return Sent::Taken;
    }
    let why = match (&outcome.error, outcome.rejected.as_slice()) {
        (Some(err), _) => err.clone(),
        (None, rejected) => {
            let reasons: Vec<&str> = rejected.iter().map(|(_, r)| r.as_str()).collect();
            format!("GitHub refused comments: {}", reasons.join("; "))
        }
    };
    let seen = outcome.on_github.unwrap_or(queued.seen);
    let mut rest = queued;
    outcome.keep_refused(&mut rest.drafts);
    rest.seen = seen;
    Sent::Partly(rest, why)
}

/// Whether the drafts can join your pending review: it must be on `head`.
/// One on another commit has its comments placed on that commit's lines,
/// and the drafts' line numbers are from this diff, so they're not mixed;
/// nor is the pending review discarded, since it may hold comments written
/// on GitHub.
fn reusable(pending: &PendingReview, head: &str) -> Result<(), String> {
    match &pending.commit {
        Some(commit) if commit != head => Err(format!(
            "You have a pending review on another commit ({}): finish or discard it on GitHub, then submit again",
            crate::ui::text::short_sha(commit)
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pending_review_on_another_commit_is_not_reused() {
        let on = |commit: Option<&str>| PendingReview {
            id: NodeId::new("R_1"),
            commit: commit.map(Into::into),
            comments: 0,
        };
        assert_eq!(reusable(&on(Some("abc")), "abc"), Ok(()));
        assert_eq!(reusable(&on(None), "abc"), Ok(()));
        let err = reusable(&on(Some("0123456789")), "abc").unwrap_err();
        assert!(err.contains("another commit (0123456)"), "{err}");
    }
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
        let anns = annotations(&threads, &mapped, &[], &HashSet::new());
        assert!(matches!(anns[0].place, Place::Line { at, .. } if at.line == 3));
        assert!(matches!(anns[1].place, Place::Moved(at) if at.line == 12));
        assert_eq!(anns[2].place, Place::Outdated(Some(3)));
        assert_eq!(outdated_to_map(&threads).len(), 2);
    }

    #[test]
    fn drafts_become_ranges_or_file_comments() {
        let right = |line| LinePos {
            side: Side::Right,
            line,
        };
        let range = Comment::Line {
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

        let single = Comment::Line {
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

        let file = Comment::File {
            path: "a.rs".into(),
            reason: None,
        };
        assert_eq!(draft(&file, "x", 3, "head").unwrap().line, None);
        assert!(
            draft(
                &Comment::Reply {
                    thread_id: NodeId::new("t")
                },
                "x",
                4,
                "h"
            )
            .is_none()
        );

        let anns = annotations(&[], &HashMap::new(), &[d_with_error()], &HashSet::new());
        assert!(anns[0].is_draft());
        assert_eq!(anns[0].error.as_deref(), Some("nope"));
    }

    fn d_with_error() -> DraftComment {
        DraftComment {
            id: 9,
            path: "a.rs".into(),
            body: "b".into(),
            line: Some(1),
            commit: "h".into(),
            error: Some("nope".into()),
            ..Default::default()
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
            crate::theme::DEFAULT_SEED,
            crate::theme::Mode::Dark,
            crate::theme::ColorDepth::TrueColor,
        );
        let mut dialog = SubmitDialog::new(&theme, PrRef::parse("o/r#1").unwrap());
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
