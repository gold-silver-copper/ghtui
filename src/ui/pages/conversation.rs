//! Conversations: issues and discussions.

use super::*;
use crate::api::browse::DiscussionComment;
use crate::api::edit::Reaction;
use crate::ui::page::ReviewLink;
use std::collections::HashSet;

/// What the comment boxes of a conversation share.
pub(super) struct Conversation<'a> {
    pub(super) base: LinkBase,
    /// Who opened it.
    pub(super) op: &'a str,
    pub(super) now: u64,
    /// Hidden comments (and long reviews) you unfolded, by their anchors.
    pub(super) unfolded: &'a HashSet<String>,
}

impl<'a> Conversation<'a> {
    /// A conversation in `repo` opened by `op`, its links relative to `rev`.
    pub(super) fn new(
        repo: &RepoId,
        rev: &str,
        op: &'a str,
        now: u64,
        unfolded: &'a HashSet<String>,
    ) -> Self {
        let base = LinkBase::new(repo, rev, "");
        Conversation {
            base,
            op,
            now,
            unfolded,
        }
    }

    /// The post that opened it: `╭─ author opened 3d ago ─── Author ─╮`,
    /// the Markdown body, its reactions, `╰──╯`.
    pub(super) fn opened(
        &self,
        page: &mut Page,
        verb: &str,
        when: &str,
        body: &str,
        reactions: &[Reaction],
    ) {
        let badge = self.badge(false, self.op);
        self.said_at(page, None, self.op, verb, when, body, badge, reactions);
    }

    /// A comment's badge: New when it came in since you started looking,
    /// else Author when it's by whoever opened the conversation.
    fn badge(&self, new: bool, author: &str) -> Option<Seg> {
        if new {
            Some(chip("New", Bg::TertiaryContainer))
        } else {
            (author == self.op).then(|| chip("Author", Bg::SecondaryContainer))
        }
    }

    /// A comment box, named `anchor` for links to it, with `chip` on its
    /// right (Author, Answer).
    #[expect(clippy::too_many_arguments, reason = "the parts of a comment box")]
    pub(super) fn said_at(
        &self,
        page: &mut Page,
        anchor: Option<String>,
        author: &str,
        verb: &str,
        when: &str,
        body: &str,
        chip: Option<Seg>,
        reactions: &[Reaction],
    ) {
        if let Some(anchor) = anchor {
            page.anchor(anchor);
        }
        let start = page.lines.len();
        let who = login_seg(page, author, Role::Strong);
        let ago = time::ago_iso(when, self.now);
        let right = Vec::from_iter(chip);
        page.box_top(vec![who, meta(format!(" {verb} {ago}"))], right);
        if body.trim().is_empty() {
            empty_row(page, "No description provided.");
        } else {
            markdown::render(page, body, Some(&self.base), Frame::Body);
        }
        reaction_chips(page, reactions);
        page.box_bottom();
        // The whole comment is a row: Enter quote-replies, as GitHub's `r` does.
        let quote = page.link(Link::Quote {
            author: author.to_owned(),
            at: when.to_owned(),
            body: body.to_owned(),
        });
        page.item(start, quote);
    }

    /// A comment, badged when it's `new` (it came in since you started
    /// looking), or else when it's by whoever opened the conversation.
    pub(super) fn comment(&self, page: &mut Page, c: &Comment, new: bool) {
        let anchor = c.id.map(|id| format!("issuecomment-{id}"));
        let badge = self.badge(new, &c.author);
        self.said_comment(page, anchor, c, "commented", badge);
    }

    /// A comment's box; a hidden one folded, as GitHub shows it, saying
    /// why, until it's unfolded (its anchor names it), and then saying
    /// it's hidden under it.
    pub(super) fn said_comment(
        &self,
        page: &mut Page,
        anchor: Option<String>,
        c: &Comment,
        verb: &str,
        badge: Option<Seg>,
    ) {
        let hidden = c.hiding.reason.as_deref().map(|r| match r.trim() {
            "" => "Hidden".to_owned(),
            r => format!("Hidden as {}", r.replace('_', "-").to_lowercase()),
        });
        let reactions = c.editable.as_ref().map_or(&[][..], |e| &e.reactions);
        let (author, at, text) = (&c.author, &c.created_at, &c.body);
        let (Some(hidden), Some(anchor)) = (hidden, anchor.clone()) else {
            self.said_at(page, anchor, author, verb, at, text, badge, reactions);
            return;
        };
        let fold = Link::Review(ReviewLink::Fold(anchor.clone()));
        if self.unfolded.contains(&anchor) {
            self.said_at(page, Some(anchor), author, verb, at, text, badge, reactions);
            item(page, fold, |page, link| {
                let segs = vec![
                    Seg::linked(format!("▾ {hidden}"), Role::Link, link),
                    meta(" · ↵ folds it"),
                ];
                page.add(Frame::None, 2, segs, Vec::new());
            });
            return;
        }
        page.anchor(anchor);
        item(page, fold, |page, link| {
            let who = login_seg(page, &c.author, Role::Strong);
            let ago = time::ago_iso(&c.created_at, self.now);
            page.box_top(vec![who, meta(format!(" {verb} {ago}"))], Vec::new());
            let segs = vec![
                Seg::linked(format!("▸ {hidden}"), Role::Link, link),
                meta(" · ↵ shows it"),
            ];
            body(page, segs);
            page.box_bottom();
        });
    }
}

/// "Locked as off-topic".
pub(super) fn locked_said(locked: crate::api::manage::Locked) -> String {
    match locked.reason {
        Some(reason) => format!("Locked as {}", reason.said()),
        None => "Locked".to_owned(),
    }
}

/// The reactions under a comment, as GitHub shows them: each one's emoji
/// and count, the ones you gave marked.
pub(super) fn reaction_chips(page: &mut Page, reactions: &[Reaction]) {
    if reactions.is_empty() {
        return;
    }
    let mut segs = Vec::new();
    for r in reactions {
        if !segs.is_empty() {
            segs.push(space());
        }
        let text = format!("{} {}", r.content.emoji(), r.count);
        segs.push(chip(text, crate::ui::chips::reaction_fill(r.mine)));
    }
    body(page, Vec::new());
    body(page, segs);
}

/// The line between timeline entries.
pub(super) fn connector(page: &mut Page) {
    page.add(Frame::None, 3, vec![meta("│")], Vec::new());
}

/// The comment box at the end of a conversation.
pub(super) fn add_comment(page: &mut Page, keys: Keys<'_>) {
    let press = format!(
        "Press {} to write a comment (Markdown; ctrl-e for $EDITOR)",
        keys.comment
    );
    comment_box(page, "Add a comment", press);
}

/// The box to write in at a conversation's end: `title`, and the key
/// that opens it.
fn comment_box(page: &mut Page, title: &str, press: String) {
    connector(page);
    item(page, Link::Comment, |page, link| {
        page.box_top(vec![Seg::linked(title, Role::Strong, link)], Vec::new());
        body(page, vec![Seg::linked(press, Role::Meta, link)]);
        page.box_bottom();
    });
}

/// The sidebar's assignees.
pub(super) fn assignees(side: &mut Page, list: &Capped<String>) {
    let more = (list.left_out() > 0).then(|| left_out_text(list.left_out(), "more", "more"));
    people(side, "Assignees", list, more, "No one assigned");
}

/// A discussion's row: its title, whether it's answered, and what it is;
/// `place` goes before its number (a search's `o/r`), and a list shows
/// its upvotes.
pub(super) fn discussion_row(
    page: &mut Page,
    target: String,
    (place, upvotes): (&str, bool),
    d: &crate::api::browse::DiscussionSummary,
    now: u64,
) {
    item(page, target, |page, link| {
        let mut segs = vec![Seg::linked(d.title.clone(), Role::Strong, link)];
        if d.answered {
            tag(&mut segs, "✓ Answered", Bg::SuccessContainer);
        }
        body(page, segs);
        let up = if upvotes {
            format!(" · ▲ {}", d.upvotes)
        } else {
            String::new()
        };
        let subline = format!(
            "{place}#{} · {} · {} · {}{up} · updated {}",
            d.number,
            d.category,
            d.author,
            plural(d.comments, "comment"),
            time::ago_iso(&d.updated_at, now)
        );
        body(page, vec![meta(subline)]);
    });
}

/// The sidebar's list of people; `more` says who else there may be.
pub(super) fn people(
    page: &mut Page,
    heading: &str,
    logins: &[String],
    more: Option<String>,
    none: &str,
) {
    aside_heading(page, heading);
    if logins.is_empty() && more.is_none() {
        say(page, none, Role::Meta);
    }
    for login in logins {
        let login = login_seg(page, login, Role::Link);
        page.wrapped(vec![login], 0, Frame::None);
    }
    if let Some(more) = more {
        say(page, more, Role::Meta);
    }
}

/// For people found in a conversation's comments, when its earlier ones
/// weren't loaded.
pub(super) fn maybe_others(earlier: bool) -> Option<String> {
    earlier.then(|| "and maybe others, earlier on GitHub".to_owned())
}

/// The sidebar's milestone, linked to its page.
pub(super) fn milestone_aside(page: &mut Page, repo: &RepoId, m: Option<&MilestoneRef>) {
    aside_heading(page, "Milestone");
    match m {
        Some(m) => {
            let target = format!("{}/milestone/{}", url::repo(repo), m.number);
            let title = link_seg(page, m.title.clone(), target, Role::Link);
            page.wrapped(vec![title], 0, Frame::None);
        }
        None => page.line(vec![meta("No milestone")]),
    }
}

pub(super) fn label_list(page: &mut Page, l: &Capped<Label>) {
    aside_heading(page, "Labels");
    if l.is_empty() {
        page.line(vec![meta("None yet")]);
    } else {
        let mut segs = Vec::new();
        labels(&mut segs, l);
        page.wrapped(segs, 0, Frame::None);
    }
}

/// A conversation's title, then its number.
fn numbered_title(page: &mut Page, title: &str, number: u64) {
    let segs = vec![Seg::new(title, Role::Title), meta(format!("  #{number}"))];
    page.wrapped(segs, 0, Frame::None);
}

pub fn issue(
    page: &mut Page,
    d: &IssueDetail,
    projects: Fetched<'_, Capped<crate::api::project::InProject>>,
    marks: Marks<'_>,
    cx: PageCtx<'_>,
) {
    let PageCtx { icons, keys, now } = cx;
    let participants = unique(std::iter::once(&d.author).chain(d.comments().map(|c| &c.author)));
    let aside = page.build_aside(|a| {
        assignees(a, &d.assignees);
        label_list(a, &d.labels);
        milestone_aside(a, &d.repo, d.milestone.as_ref());
        timeline::relations_aside(a, icons, d);
        timeline::projects_aside(a, projects);
        let earlier = d.timeline.left_out() > 0;
        people(a, "Participants", &participants, maybe_others(earlier), "");
    });
    numbered_title(page, &d.title, d.number);
    let author = login_seg(page, &d.author, Role::Strong);
    let mut segs = vec![
        state_chip(Status::Issue(d.state), icons),
        Seg::new("  ", Role::Body),
        author,
        meta(format!(
            " opened this issue {} · {}",
            time::ago_iso(&d.created_at, now),
            plural(d.comment_count, "comment")
        )),
    ];
    if d.pinned {
        segs.push(Seg::new("  ", Role::Body));
        segs.push(chip("Pinned", Bg::SecondaryContainer));
    }
    if let Some(locked) = d.locked {
        segs.push(Seg::new("  ", Role::Body));
        segs.push(chip(locked_said(locked), Bg::TertiaryContainer));
    }
    if !aside {
        labels(&mut segs, &d.labels);
        if let Some(m) = &d.milestone {
            let target = format!("{}/milestone/{}", url::repo(&d.repo), m.number);
            segs.push(meta("  "));
            segs.push(link_seg(page, format!("◷ {}", m.title), target, Role::Link));
        }
    }
    page.wrapped(segs, 0, Frame::None);
    if aside || d.assignees.is_empty() {
        page.rule(0, Frame::None);
    }
    if !aside && !d.assignees.is_empty() {
        let mut segs = vec![meta("Assignees: ")];
        segs.extend(login_list(page, ", ", &d.assignees));
        more_chips(&mut segs, &d.assignees);
        page.line(segs);
    }
    if !aside {
        timeline::relations_inline(page, icons, d);
        timeline::projects_inline(page, projects);
    }
    let talk = Conversation::new(&d.repo, "HEAD", &d.author, now, marks.unfolded);
    let reactions = &d.editable.reactions;
    talk.opened(page, "opened", &d.created_at, &d.body, reactions);
    earlier_here(page, &d.timeline, "comment or event", "comments and events");
    let place = timeline::Place {
        repo: &d.repo,
        pr: None,
        threads: HashMap::new(),
        icons,
    };
    timeline::entries(page, &talk, &place, &d.timeline, marks);
    add_comment(page, keys);
}

/// A list of discussions: the categories to filter by (`base` is the
/// list's URL), then the discussions, most recently updated first.
pub fn discussions(
    page: &mut Page,
    base: &str,
    list: Fetched<'_, DiscussionList>,
    category: Option<&str>,
    now: u64,
) {
    let Some(l) = list.show(page, "discussions") else { return };
    if !l.categories.is_empty() {
        let all = ("All".to_owned(), base.to_owned(), category.is_none());
        let categories = l.categories.iter().map(|c| {
            let target = format!("{base}/categories/{}", c.slug);
            (c.name.clone(), target, category == Some(c.slug.as_str()))
        });
        let tabs = std::iter::once(all).chain(categories);
        let mut segs = nav_strip(page, &meta(" · "), &Role::Link, tabs);
        more_chips(&mut segs, &l.categories);
        page.wrapped(segs, 0, Frame::None);
        page.blank();
    }
    let empty = "No discussions yet.";
    counted_box(
        page,
        "Discussions",
        Vec::new(),
        &l.results,
        empty,
        |page, d| {
            discussion_row(page, format!("{base}/{}", d.number), ("", true), d, now);
        },
    );
}

/// A discussion: its post, the answer at the top as GitHub shows it, then
/// its comments (the answer marked) with their replies, each named for
/// links to it (`#discussioncomment-…`), and the box to write in. Those
/// in `fresh` (by GitHub's number) came in since you started looking.
pub fn discussion(
    page: &mut Page,
    d: &DiscussionDetail,
    fresh: &HashSet<u64>,
    unfolded: &HashSet<String>,
    cx: PageCtx<'_>,
) {
    numbered_title(page, &d.title, d.number);
    let mut segs = vec![chip(d.category.clone(), Bg::SecondaryContainer)];
    if d.answered {
        tag(&mut segs, "✓ Answered", Bg::SuccessContainer);
    }
    match d.closed {
        Some(Some(reason)) => {
            segs.push(space());
            let said = format!("Closed as {}", reason.said());
            segs.push(chip(said, Bg::ContainerHigh));
        }
        Some(None) => {
            tag(&mut segs, "Closed", Bg::ContainerHigh);
        }
        None => {}
    }
    if let Some(locked) = d.locked {
        tag(&mut segs, locked_said(locked), Bg::TertiaryContainer);
    }
    segs.push(meta(format!(
        "  ▲ {} · {}",
        d.upvotes,
        plural(d.comments.total(), "comment")
    )));
    page.wrapped(segs, 0, Frame::None);
    page.rule(0, Frame::None);
    let talk = Conversation::new(&d.repo, "HEAD", &d.author, cx.now, unfolded);
    let reactions = &d.editable.reactions;
    talk.opened(page, "started", &d.created_at, &d.body, reactions);
    let anchor = |c: &Comment| c.id.map(|id| format!("discussioncomment-{id}"));
    let new = |c: &Comment| c.id.is_some_and(|id| fresh.contains(&id));
    let comment = |page: &mut Page, c: &DiscussionComment, anchored: bool| {
        let new = anchored && new(&c.comment);
        let badge = if c.answer && !new {
            Some(chip("✓ Answer", Bg::SuccessContainer))
        } else {
            talk.badge(new, &c.comment.author)
        };
        let verb = if d.answerable {
            "answered"
        } else {
            "commented"
        };
        let said = &c.comment;
        let verb = format!("{verb} · ▲ {}", c.upvotes);
        let anchor = anchored.then(|| anchor(said)).flatten();
        talk.said_comment(page, anchor, said, &verb, badge);
    };
    // The answer, at the top; it's in its place below too.
    if let Some(answer) = &d.answer {
        connector(page);
        let mut segs = vec![Seg::new("✓ ", Role::Success)];
        match &d.answer_by {
            Some(by) => {
                segs.push(meta("Answer selected by "));
                segs.push(login_seg(page, by, Role::Strong));
            }
            None => segs.push(meta("Answer selected")),
        }
        page.add(Frame::None, 2, segs, Vec::new());
        comment(page, answer, false);
    }
    for c in &d.comments {
        connector(page);
        comment(page, c, true);
        for r in &c.replies {
            let badge = talk.badge(new(r), &r.author);
            talk.said_comment(page, anchor(r), r, "replied", badge);
        }
        more_here(page, &c.replies, "reply", "replies");
    }
    more_here(page, &d.comments, "comment", "comments");
    // Where to write: an answer, in a category that takes them.
    let (title, what) = if d.answerable {
        ("Write an answer", "an answer")
    } else {
        ("Add a comment", "a comment")
    };
    let mut press = format!(
        "Press {} to write {what}, or on a comment to reply in its thread",
        cx.keys.comment
    );
    if d.locked.is_some() {
        press.push_str(" (it's locked: only those with write access can)");
    }
    comment_box(page, title, press);
}
