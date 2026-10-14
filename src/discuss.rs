//! Writing in a discussion, from its page: a comment (an answer, in a
//! category that takes them) or a reply in a comment's thread, editing or
//! deleting one, reacting, and marking the answer. What a key does is
//! decided here, by the comment selected (the post, a comment or a
//! reply); each change goes to GitHub as a [`Change::Discussion`] (or, a
//! reaction, [`Edit::React`]), and the page follows GitHub until it shows
//! it ([`shows`], asked by [`crate::act::follow`]).

use crate::api::browse::{Comment, DiscussionComment, DiscussionDetail, DiscussionsOf};
use crate::api::change::Change;
use crate::api::discussion::Discuss;
use crate::api::edit::{Edit, Editable};
use crate::api::model::NodeId;
use crate::route::{Route, discussions};

use crate::act::{By, Confirm, Subject};
use crate::compose;
use crate::data::DataKey;
use crate::edit::Choosing;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::state::{Cmd, Overlay, Screen, State};
use crate::ui::page::Link;

/// The actions a discussion's page takes as its own.
pub const ACTIONS: [Action; 6] = [
    Action::Comment,
    Action::QuoteReply,
    Action::React,
    Action::MarkAnswer,
    Action::EditComment,
    Action::DeleteComment,
];

/// What the composer writes in a discussion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Writing {
    pub of: DiscussionsOf,
    pub number: u64,
    pub writes: Writes,
    /// The composer's title: "Reply to hubot".
    pub title: String,
    /// When the comment it's about was written: a reply goes under it,
    /// an edit in its place.
    pub at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Writes {
    /// A comment on the discussion, or a reply in `reply_to`'s thread.
    Comment {
        discussion: NodeId,
        reply_to: Option<NodeId>,
    },
    /// New text for the post (`post`) or a comment.
    Edit { id: NodeId, post: bool },
}

impl Writing {
    /// Where the composer goes: in place of the comment `at` names, or
    /// under it (`true`); without one, the box to write in.
    pub fn place(&self) -> (Option<&String>, bool) {
        let under = matches!(self.writes, Writes::Comment { .. });
        (self.at.as_ref(), under)
    }

    /// What `ctrl-s` does, for the composer's hint.
    pub fn save(&self) -> &'static str {
        match self.writes {
            Writes::Comment { reply_to: None, .. } => "post comment",
            Writes::Comment { .. } => "post reply",
            Writes::Edit { .. } => "save",
        }
    }
}

/// Sends what the composer wrote.
#[must_use]
pub fn send(w: &Writing, body: String) -> Vec<Cmd> {
    let change = match &w.writes {
        Writes::Comment {
            discussion,
            reply_to,
        } => Discuss::Comment {
            discussion: discussion.clone(),
            reply_to: reply_to.clone(),
            body,
        },
        Writes::Edit { id, post } => Discuss::Edit {
            id: id.clone(),
            post: *post,
            body,
        },
    };
    let about = Subject::Discussion(w.of.clone(), w.number);
    vec![crate::act::send(
        Change::Discussion(change),
        By::Edit(about),
    )]
}

/// The discussion on screen, as loaded, and the comment selected.
struct Here<'a> {
    of: &'a DiscussionsOf,
    number: u64,
    d: Option<&'a DiscussionDetail>,
    /// The selected comment's author and time, as its row names it.
    selected: Option<(&'a str, &'a str)>,
}

fn here(state: &State) -> Option<Here<'_>> {
    let Screen::Page(p) = state.screen() else {
        return None;
    };
    let Route::Discussions(discussions::Discussion { of, number }) = &p.route else {
        return None;
    };
    let d = state.picked::<DiscussionDetail>(&DataKey::Discussion(of.clone(), *number));
    let selected = match p.selected_link() {
        Some(Link::Quote { author, at, .. }) => Some((author.as_str(), at.as_str())),
        _ => None,
    };
    Some(Here {
        of,
        number: *number,
        d,
        selected,
    })
}

/// Whether `action` is the discussion's on screen to do.
/// What discussions add: their own actions on a discussion's pages.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    takes,
    act,
    offers: doables,
    ..crate::features::Feature::NONE
};

fn takes(state: &State, action: Action) -> bool {
    ACTIONS.contains(&action) && here(state).is_some()
}

/// What's selected in a discussion.
enum Spot<'a> {
    Post,
    Comment(&'a DiscussionComment),
    /// A reply, in its comment's thread.
    Reply(&'a DiscussionComment, &'a Comment),
}

/// The post, comment or reply written by `author` at `at`.
fn spot<'a>(d: &'a DiscussionDetail, (author, at): (&str, &str)) -> Option<Spot<'a>> {
    let is = |c: &Comment| c.author == author && c.created_at == at;
    if d.author == author && d.created_at == at {
        return Some(Spot::Post);
    }
    let comments = d.answer.as_deref().into_iter().chain(&d.comments);
    for c in comments {
        if is(&c.comment) {
            return Some(Spot::Comment(c));
        }
        if let Some(r) = c.replies.iter().find(|r| is(r)) {
            return Some(Spot::Reply(c, r));
        }
    }
    None
}

impl Spot<'_> {
    fn comment(&self) -> Option<&Comment> {
        match self {
            Spot::Post => None,
            Spot::Comment(c) => Some(&c.comment),
            Spot::Reply(_, r) => Some(r),
        }
    }

    /// The thread a reply to it goes in: its own, or its comment's.
    fn thread(&self) -> Option<&DiscussionComment> {
        match self {
            Spot::Post => None,
            Spot::Comment(c) | Spot::Reply(c, _) => Some(c),
        }
    }

    fn name(&self, d: &DiscussionDetail) -> String {
        match self.comment() {
            None => "the post".to_owned(),
            Some(c) if c.author == d.author && matches!(self, Spot::Reply(..)) => {
                format!("{}'s reply", c.author)
            }
            Some(c) => format!("{}'s comment", c.author),
        }
    }
}

/// What an action does here.
enum Plan {
    Write(Writing, String),
    React(Editable),
    Send(Discuss),
    Delete(Box<Confirm>),
}

fn editable(c: &Comment) -> Result<&Editable, String> {
    c.editable
        .as_ref()
        .ok_or_else(|| "GitHub didn't say what you may do with it".to_owned())
}

fn plan(state: &State, h: &Here<'_>, action: Action) -> Result<Plan, String> {
    let d = h.d.ok_or("The discussion hasn't loaded yet")?;
    let selected = h.selected.and_then(|s| spot(d, s));
    let next = state.first_key(Action::NextItem);
    let select = || format!("Select a comment first: {next} goes comment by comment");
    let write = |writes, title: String, at: Option<String>| Writing {
        of: h.of.clone(),
        number: h.number,
        writes,
        title,
        at,
    };
    match action {
        Action::Comment | Action::QuoteReply => {
            let quoted = match (action, &selected) {
                (Action::QuoteReply, None) => return Err(select()),
                (Action::QuoteReply, Some(s)) => {
                    let (author, body) = match s.comment() {
                        Some(c) => (&c.author, &c.body),
                        None => (&d.author, &d.body),
                    };
                    let lines: String = body.trim().lines().map(|l| format!("> {l}\n")).collect();
                    format!("> @{author} wrote:\n{lines}\n")
                }
                _ => String::new(),
            };
            let what = if d.answerable { "Answer" } else { "Comment" };
            Ok(
                match selected.as_ref().and_then(|s| Some((s, s.thread()?))) {
                    Some((s, thread)) => {
                        let id = editable(&thread.comment)?.id.clone();
                        let replied = s.comment().map(|c| c.author.clone()).unwrap_or_default();
                        let at = s.comment().map(|c| c.created_at.clone());
                        let writes = Writes::Comment {
                            discussion: d.id.clone(),
                            reply_to: Some(id),
                        };
                        Plan::Write(write(writes, format!("Reply to {replied}"), at), quoted)
                    }
                    None => {
                        let writes = Writes::Comment {
                            discussion: d.id.clone(),
                            reply_to: None,
                        };
                        let title = format!("{what} on “{}”", d.title);
                        Plan::Write(write(writes, title, None), quoted)
                    }
                },
            )
        }
        Action::React => {
            let target = match selected.as_ref().and_then(Spot::comment) {
                Some(c) => editable(c)?,
                None => &d.editable,
            };
            if !target.can_react {
                return Err("GitHub doesn't let you react to it".into());
            }
            Ok(Plan::React(target.clone()))
        }
        Action::MarkAnswer => {
            let s = selected.ok_or_else(select)?;
            let c = match s {
                Spot::Comment(c) => c,
                Spot::Post => {
                    return Err("The post can't be its own answer: select a comment".into());
                }
                Spot::Reply(..) => {
                    return Err("A reply can't be the answer, only a comment".into());
                }
            };
            if !d.answerable {
                return Err(format!(
                    "{} doesn't take answers: only a Q&A category's discussions have one",
                    d.category
                ));
            }
            let comment = editable(&c.comment)?.id.clone();
            match (c.answer, c.can_unmark, c.can_mark) {
                (true, true, _) => Ok(Plan::Send(Discuss::Answer { comment, on: false })),
                (true, false, _) => Err(
                    "GitHub doesn't let you unmark the answer: that takes the discussion's author or triage access"
                        .into(),
                ),
                (false, _, true) => Ok(Plan::Send(Discuss::Answer { comment, on: true })),
                (false, _, false) => Err(
                    "GitHub doesn't let you mark the answer: that takes the discussion's author or triage access"
                        .into(),
                ),
            }
        }
        Action::EditComment => {
            let s = selected.unwrap_or(Spot::Post);
            let name = s.name(d);
            let (id, post, body, at, can) = match s.comment() {
                Some(c) => {
                    let e = editable(c)?;
                    (e.id.clone(), false, &c.body, &c.created_at, e.can_update)
                }
                None => (
                    d.id.clone(),
                    true,
                    &d.body,
                    &d.created_at,
                    d.editable.can_update,
                ),
            };
            if !can {
                return Err(format!(
                    "GitHub doesn't let you edit {name}: it takes its author or write access"
                ));
            }
            let writes = Writes::Edit { id, post };
            Ok(Plan::Write(
                write(writes, format!("Edit {name}"), Some(at.clone())),
                body.clone(),
            ))
        }
        Action::DeleteComment => {
            let s = selected.ok_or_else(select)?;
            let Some(c) = s.comment() else {
                return Err(
                    "The discussion itself isn't deleted here: o opens it on GitHub".into(),
                );
            };
            let e = editable(c)?;
            let name = s.name(d);
            if !e.can_delete {
                return Err(format!(
                    "GitHub doesn't let you delete {name}: it takes its author or write access"
                ));
            }
            let mut facts = Vec::new();
            if let Some(line) = c.body.lines().find(|l| !l.trim().is_empty()) {
                let line = crate::ui::text::truncate(line.trim(), 60);
                facts.push((format!("“{line}”"), false));
            }
            if let Spot::Comment(c) = s
                && !c.replies.is_empty()
            {
                facts.push(("Its replies go with it".to_owned(), true));
            }
            facts.push(("Deleting it can't be undone".to_owned(), true));
            let delete = Change::Discussion(Discuss::Delete { id: e.id.clone() });
            Ok(Plan::Delete(Box::new(Confirm::new(
                action,
                Subject::Discussion(h.of.clone(), h.number),
                format!("Delete {name}?"),
                facts,
                vec![("Delete".into(), delete)],
            ))))
        }
        _ => Err(action.not_here()),
    }
}

/// Runs one of [`ACTIONS`] on the discussion on screen.
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    let Some(h) = here(state) else {
        state.info(action.not_here());
        return Vec::new();
    };
    let about = Subject::Discussion(h.of.clone(), h.number);
    let planned = plan(state, &h, action).and_then(|plan| match crate::act::awaiting_here(state) {
        Some(_) => Err("GitHub hasn't shown the last change yet: wait a moment".to_owned()),
        None => Ok(plan),
    });
    match planned {
        Err(why) => {
            state.info(why);
            Vec::new()
        }
        Ok(Plan::Write(w, text)) => state.compose(compose::Target::Discussion(w), &text),
        Ok(Plan::React(target)) => {
            let choosing = Choosing::React { about, target };
            state.open_picker(crate::picker::Kind::Edit(choosing))
        }
        Ok(Plan::Send(change)) => {
            state.info(format!("{}…", doing(&change)));
            vec![crate::act::send(Change::Discussion(change), By::Act(about))]
        }
        Ok(Plan::Delete(confirm)) => {
            state.overlay = Some(Overlay::Confirm(confirm));
            Vec::new()
        }
    }
}

/// The menu's rows on a discussion's page.
pub fn doables(state: &State, out: &mut Vec<Doable>) {
    let Some(h) = here(state) else {
        return;
    };
    let answerable = h.d.is_some_and(|d| d.answerable);
    let replying = h.selected.is_some_and(|s| {
        h.d.and_then(|d| spot(d, s))
            .is_some_and(|s| !matches!(s, Spot::Post))
    });
    let comment = match (replying, answerable) {
        (true, _) => "Reply in the comment's thread",
        (false, true) => "Write an answer",
        (false, false) => "Write a comment",
    };
    let rows = [
        (Action::Comment, "This page", comment, "comment"),
        (
            Action::QuoteReply,
            "This page",
            "Reply quoting the selected comment",
            "",
        ),
        (Action::EditComment, "Edit", "Edit the comment or post", ""),
        (Action::DeleteComment, "Edit", "Delete the comment", ""),
        (Action::React, "Edit", "React to the comment or post", ""),
        (
            Action::MarkAnswer,
            "Change",
            "Mark or unmark the answer",
            "answer",
        ),
    ];
    for (action, section, label, short) in rows {
        if action == Action::MarkAnswer && !answerable {
            continue;
        }
        let why = plan(state, &h, action).err();
        out.push(crate::nav::row(section, action, label, short, why));
    }
}

/// Every comment and reply loaded, and the post's own.
fn all(d: &DiscussionDetail) -> impl Iterator<Item = (&Editable, &str)> {
    let comments = d.comments.iter().flat_map(|c| {
        std::iter::once(&c.comment)
            .chain(c.replies.iter())
            .filter_map(|c| Some((c.editable.as_ref()?, c.body.as_str())))
    });
    std::iter::once((&d.editable, d.body.as_str())).chain(comments)
}

/// The same text, but for line endings and the ends' whitespace.
fn same(a: &str, b: &str) -> bool {
    a.replace("\r\n", "\n").trim() == b.replace("\r\n", "\n").trim()
}

/// Whether the discussion, as loaded, shows `change`.
pub fn shows(state: &State, change: &Change, of: &DiscussionsOf, number: u64) -> bool {
    let key = DataKey::Discussion(of.clone(), number);
    let Some(d) = state.picked::<DiscussionDetail>(&key) else {
        return false;
    };
    let find = |id: &NodeId| all(d).find(|(e, _)| e.id == *id);
    match change {
        Change::Discussion(Discuss::Comment { body, reply_to, .. }) => {
            let me = state.viewer.as_deref();
            let mine = |c: &Comment| Some(c.author.as_str()) == me && same(&c.body, body);
            // Past the comments loaded, it can't be seen: don't wait.
            let more = match reply_to {
                Some(_) => d.comments.iter().any(|c| c.replies.left_out() > 0),
                None => d.comments.left_out() > 0,
            };
            more || d
                .comments
                .iter()
                .any(|c| mine(&c.comment) || c.replies.iter().any(mine))
        }
        Change::Discussion(Discuss::Edit { id, body, .. }) => {
            find(id).is_none_or(|(_, now)| same(now, body))
        }
        Change::Discussion(Discuss::Delete { id }) => find(id).is_none(),
        Change::Discussion(Discuss::Answer { comment, on }) => {
            let marked = d.answer.as_ref().and_then(|a| a.comment.editable.as_ref());
            marked.is_some_and(|e| e.id == *comment) == *on
        }
        Change::Edit(Edit::React {
            subject,
            content,
            on,
        }) => find(subject).is_none_or(|(e, _)| e.reacted(*content) == *on),
        _ => true,
    }
}

/// What a change is doing while it's sent.
pub fn doing(change: &Discuss) -> &'static str {
    match change {
        Discuss::Comment { .. } => "Posting",
        Discuss::Edit { .. } => "Saving",
        Discuss::Delete { .. } => "Deleting it",
        Discuss::Answer { on: true, .. } => "Marking the answer",
        Discuss::Answer { .. } => "Unmarking the answer",
    }
}

/// What a change did, once GitHub said so.
pub fn done(change: &Discuss) -> &'static str {
    match change {
        Discuss::Comment { reply_to: None, .. } => "Comment posted",
        Discuss::Comment { .. } => "Reply posted",
        Discuss::Edit { .. } => "Saved",
        Discuss::Delete { .. } => "Deleted",
        Discuss::Answer { on: true, .. } => "Marked as the answer",
        Discuss::Answer { .. } => "No longer the answer",
    }
}

/// What was refused, for "GitHub refused …".
pub fn what(change: &Discuss) -> &'static str {
    match change {
        Discuss::Comment { reply_to: None, .. } => "the comment",
        Discuss::Comment { .. } => "the reply",
        Discuss::Edit { .. } => "the edit",
        Discuss::Delete { .. } => "deleting it",
        Discuss::Answer { on: true, .. } => "marking the answer",
        Discuss::Answer { .. } => "unmarking the answer",
    }
}
