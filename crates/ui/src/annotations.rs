//! Review threads and draft comments as shown in the diff.
//!
//! Both GitHub threads and local drafts become [`Annotation`]s anchored by
//! file path, side and line number (never by row). Line annotations show a
//! gutter marker and expand in place below their line; file-level comments,
//! and outdated threads that can't be mapped onto the current diff, sit
//! under the file header.

use ghtui_diff::anchor::LinePos;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum AnnotationKey {
    Thread(ghtui_api::model::NodeId),
    Draft(u64),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnnotationComment {
    pub author: String,
    pub body: String,
    /// ISO 8601; empty for drafts.
    pub created_at: String,
    /// In the viewer's unsubmitted review on GitHub.
    pub pending: bool,
}

/// Where an annotation sits in the current diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// On the whole file.
    File { outdated: bool },
    /// On a current line, or lines `start..=at.line` when `start` is set.
    Line { at: LinePos, start: Option<u32> },
    /// Outdated, but its line still exists unchanged and it was moved there.
    Moved(LinePos),
    /// Outdated and not mapped onto the current diff, with the line it was
    /// written on, if known.
    Outdated(Option<u32>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Annotation {
    pub key: AnnotationKey,
    pub path: String,
    pub place: Place,
    pub resolved: bool,
    pub comments: Vec<AnnotationComment>,
    /// Replies on GitHub between the first comment and the rest, not
    /// fetched (a thread brings its first comment and its newest 99).
    pub left_out: u64,
    /// For drafts: GitHub's reason for rejecting it last time.
    pub error: Option<String>,
    pub can_reply: bool,
    pub can_resolve: bool,
    pub can_unresolve: bool,
}

impl Annotation {
    pub fn is_draft(&self) -> bool {
        matches!(self.key, AnnotationKey::Draft(_))
    }

    /// Placed on a line (as opposed to under the file header).
    pub fn on_line(&self) -> Option<LinePos> {
        match self.place {
            Place::Line { at, .. } | Place::Moved(at) => Some(at),
            Place::File { .. } | Place::Outdated(_) => None,
        }
    }

    /// Expanded unless resolved (drafts are always open).
    pub fn open_by_default(&self) -> bool {
        self.is_draft() || !self.resolved
    }
}

/// What a thread row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadRowKind {
    /// A collapsed thread under a file header: one summary line.
    Summary,
    /// A comment's author line; `first` also carries the thread's state.
    Head {
        comment: usize,
        first: bool,
    },
    Body,
    /// The replies left out, after the first comment.
    Gap,
    /// Why GitHub rejected a draft.
    Error,
    Footer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadRow {
    /// Index into `Doc::annotations`.
    pub ann: u32,
    pub kind: ThreadRowKind,
    pub text: String,
}
