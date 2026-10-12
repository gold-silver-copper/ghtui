//! Conversations: issues and discussions.

use super::*;
use crate::api::browse::DiscussionComment;
use crate::api::edit::Reaction;
use std::collections::HashSet;

/// What the comment boxes of a conversation share.
pub(super) struct Conversation<'a> {
    pub(super) base: LinkBase,
    /// Who opened it.
    pub(super) op: &'a str,
    pub(super) now: u64,
}

impl Conversation<'_> {
    /// A comment box: `╭─ author commented 3d ago ─── Author ─╮`, the
    /// Markdown body, its reactions, `╰──╯`. `verb` is "commented",
    /// "opened"...
    #[expect(clippy::too_many_arguments, reason = "the parts of a comment box")]
    pub(super) fn said(
        &self,
        page: &mut Page,
        author: &str,
        verb: &str,
        when: &str,
        body: &str,
        badge: bool,
        reactions: &[Reaction],
    ) {
        self.said_at(
            page,
            None,
            author,
            verb,
            when,
            body,
            badge.then(|| chip("Author", Bg::SecondaryContainer)),
            reactions,
        );
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
        page.box_top(
            vec![who, Seg::new(format!(" {verb} {ago}"), Role::Meta)],
            right,
        );
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
        let badge = if new {
            Some(chip("New", Bg::TertiaryContainer))
        } else {
            (c.author == self.op).then(|| chip("Author", Bg::SecondaryContainer))
        };
        let reactions = c.editable.as_ref().map_or(&[][..], |e| &e.reactions);
        self.said_at(
            page,
            anchor,
            &c.author,
            "commented",
            &c.created_at,
            &c.body,
            badge,
            reactions,
        );
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
    page.add(Frame::None, 3, vec![Seg::new("│", Role::Meta)], Vec::new());
}

/// The comment box at the end of a conversation.
pub(super) fn add_comment(page: &mut Page, keys: Keys<'_>) {
    connector(page);
    item(page, Link::Comment, |page, link| {
        page.box_top(
            vec![Seg::linked("Add a comment", Role::Strong, link)],
            Vec::new(),
        );
        let press = format!(
            "Press {} to write a comment (Markdown; ctrl-e for $EDITOR)",
            keys.comment
        );
        body(page, vec![Seg::linked(press, Role::Meta, link)]);
        page.box_bottom();
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
        page.wrapped(vec![Seg::new(none, Role::Meta)], 0, Frame::None);
    }
    for login in logins {
        let login = login_seg(page, login, Role::Link);
        page.wrapped(vec![login], 0, Frame::None);
    }
    if let Some(more) = more {
        page.wrapped(vec![Seg::new(more, Role::Meta)], 0, Frame::None);
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
        None => page.line(vec![Seg::new("No milestone", Role::Meta)]),
    }
}

pub(super) fn label_list(page: &mut Page, l: &Capped<Label>) {
    aside_heading(page, "Labels");
    if l.is_empty() {
        page.line(vec![Seg::new("None yet", Role::Meta)]);
    } else {
        let mut segs = Vec::new();
        labels(&mut segs, l);
        page.wrapped(segs, 0, Frame::None);
    }
}

pub fn issue(page: &mut Page, d: &IssueDetail, marks: Marks<'_>, cx: PageCtx<'_>) {
    let PageCtx { icons, keys, now } = cx;
    let participants = unique(std::iter::once(&d.author).chain(d.comments().map(|c| &c.author)));
    let aside = page.build_aside(|a| {
        people(
            a,
            "Assignees",
            &d.assignees,
            (d.assignees.left_out() > 0)
                .then(|| left_out_text(d.assignees.left_out(), "more", "more")),
            "No one assigned",
        );
        label_list(a, &d.labels);
        milestone_aside(a, &d.repo, d.milestone.as_ref());
        timeline::relations_aside(a, icons, d);
        let earlier = d.timeline.left_out() > 0;
        people(a, "Participants", &participants, maybe_others(earlier), "");
    });
    page.wrapped(
        vec![
            Seg::new(d.title.clone(), Role::Title),
            Seg::new(format!("  #{}", d.number), Role::Meta),
        ],
        0,
        Frame::None,
    );
    let author = login_seg(page, &d.author, Role::Strong);
    let mut segs = vec![
        state_chip(Status::Issue(d.state), icons),
        Seg::new("  ", Role::Body),
        author,
        Seg::new(
            format!(
                " opened this issue {} · {}",
                time::ago_iso(&d.created_at, now),
                plural(d.comment_count, "comment")
            ),
            Role::Meta,
        ),
    ];
    if !aside {
        labels(&mut segs, &d.labels);
        if let Some(m) = &d.milestone {
            let target = format!("{}/milestone/{}", url::repo(&d.repo), m.number);
            segs.push(Seg::new("  ", Role::Meta));
            segs.push(link_seg(page, format!("◷ {}", m.title), target, Role::Link));
        }
    }
    page.wrapped(segs, 0, Frame::None);
    if aside || d.assignees.is_empty() {
        page.rule(0, Frame::None);
    }
    if !aside && !d.assignees.is_empty() {
        let mut segs = vec![Seg::new("Assignees: ", Role::Meta)];
        for (i, a) in d.assignees.iter().enumerate() {
            if i > 0 {
                segs.push(Seg::new(", ", Role::Meta));
            }
            segs.push(login_seg(page, a, Role::Link));
        }
        more_chips(&mut segs, &d.assignees);
        page.line(segs);
    }
    if !aside {
        timeline::relations_inline(page, icons, d);
    }
    let talk = Conversation {
        base: LinkBase::new(&d.repo, "HEAD", ""),
        op: &d.author,
        now,
    };
    let reactions = &d.editable.reactions;
    talk.said(
        page,
        &d.author,
        "opened",
        &d.created_at,
        &d.body,
        true,
        reactions,
    );
    earlier_here(page, &d.timeline, "comment or event", "comments and events");
    let place = timeline::Place {
        repo: &d.repo,
        pr: None,
        threads: &[],
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
    let Some(l) = list.show(page, "discussions") else {
        return;
    };
    if !l.categories.is_empty() {
        let mut segs = Vec::new();
        let all = category.is_none();
        segs.push(link_seg(
            page,
            "All",
            base.to_owned(),
            if all { Role::Strong } else { Role::Link },
        ));
        for c in &l.categories {
            segs.push(Seg::new(" · ", Role::Meta));
            let target = format!("{base}/categories/{}", c.slug);
            let role = if category == Some(c.slug.as_str()) {
                Role::Strong
            } else {
                Role::Link
            };
            segs.push(link_seg(page, c.name.clone(), target, role));
        }
        more_chips(&mut segs, &l.categories);
        page.wrapped(segs, 0, Frame::None);
        page.blank();
    }
    let r = &l.results;
    let title = vec![Seg::new(
        format!("Discussions  {}", count_of(r)),
        Role::Strong,
    )];
    list_box(
        page,
        title,
        Vec::new(),
        r,
        "No discussions yet.",
        |page, d| {
            item(page, format!("{base}/{}", d.number), |page, link| {
                let mut segs = vec![Seg::linked(d.title.clone(), Role::Strong, link)];
                if d.answered {
                    segs.push(space());
                    segs.push(chip("✓ Answered", Bg::SuccessContainer));
                }
                body(page, segs);
                let meta = format!(
                    "#{} · {} · {} · {} · ▲ {} · updated {}",
                    d.number,
                    d.category,
                    d.author,
                    plural(d.comments, "comment"),
                    d.upvotes,
                    time::ago_iso(&d.updated_at, now)
                );
                body(page, vec![Seg::new(meta, Role::Meta)]);
            });
        },
    );
}

/// A discussion: its post, the answer at the top as GitHub shows it, then
/// its comments (the answer marked) with their replies, each named for
/// links to it (`#discussioncomment-…`), and the box to write in. Those
/// in `fresh` (by GitHub's number) came in since you started looking.
pub fn discussion(page: &mut Page, d: &DiscussionDetail, fresh: &HashSet<u64>, cx: PageCtx<'_>) {
    page.wrapped(
        vec![
            Seg::new(d.title.clone(), Role::Title),
            Seg::new(format!("  #{}", d.number), Role::Meta),
        ],
        0,
        Frame::None,
    );
    let mut segs = vec![chip(d.category.clone(), Bg::SecondaryContainer)];
    if d.answered {
        segs.push(space());
        segs.push(chip("✓ Answered", Bg::SuccessContainer));
    }
    if d.locked {
        segs.push(space());
        segs.push(chip("Locked", Bg::TertiaryContainer));
    }
    segs.push(Seg::new(
        format!(
            "  ▲ {} · {}",
            d.upvotes,
            plural(d.comments.total(), "comment")
        ),
        Role::Meta,
    ));
    page.wrapped(segs, 0, Frame::None);
    page.rule(0, Frame::None);
    let talk = Conversation {
        base: LinkBase::new(&d.repo, "HEAD", ""),
        op: &d.author,
        now: cx.now,
    };
    talk.said(
        page,
        &d.author,
        "started",
        &d.created_at,
        &d.body,
        true,
        &d.editable.reactions,
    );
    let anchor = |c: &Comment| c.id.map(|id| format!("discussioncomment-{id}"));
    let new = |c: &Comment| c.id.is_some_and(|id| fresh.contains(&id));
    let reactions = |c: &Comment| {
        c.editable
            .as_ref()
            .map_or(Vec::new(), |e| e.reactions.clone())
    };
    let comment = |page: &mut Page, c: &DiscussionComment, anchored: bool| {
        let badge = if new(&c.comment) && anchored {
            Some(chip("New", Bg::TertiaryContainer))
        } else if c.answer {
            Some(chip("✓ Answer", Bg::SuccessContainer))
        } else {
            (c.comment.author == d.author).then(|| chip("Author", Bg::SecondaryContainer))
        };
        let verb = if d.answerable {
            "answered"
        } else {
            "commented"
        };
        let said = &c.comment;
        talk.said_at(
            page,
            anchored.then(|| anchor(said)).flatten(),
            &said.author,
            &format!("{verb} · ▲ {}", c.upvotes),
            &said.created_at,
            &said.body,
            badge,
            &reactions(said),
        );
    };
    // The answer, at the top; it's in its place below too.
    if let Some(answer) = &d.answer {
        connector(page);
        let mut segs = vec![Seg::new("✓ ", Role::Success)];
        match &d.answer_by {
            Some(by) => {
                segs.push(Seg::new("Answer selected by ", Role::Meta));
                segs.push(login_seg(page, by, Role::Strong));
            }
            None => segs.push(Seg::new("Answer selected", Role::Meta)),
        }
        page.add(Frame::None, 2, segs, Vec::new());
        comment(page, answer, false);
    }
    for c in &d.comments {
        connector(page);
        comment(page, c, true);
        for r in &c.replies {
            let badge = if new(r) {
                Some(chip("New", Bg::TertiaryContainer))
            } else {
                (r.author == d.author).then(|| chip("Author", Bg::SecondaryContainer))
            };
            talk.said_at(
                page,
                anchor(r),
                &r.author,
                "replied",
                &r.created_at,
                &r.body,
                badge,
                &reactions(r),
            );
        }
        more_here(page, &c.replies, "reply", "replies");
    }
    more_here(page, &d.comments, "comment", "comments");
    // Where to write: an answer, in a category that takes them.
    connector(page);
    let (title, what) = if d.answerable {
        ("Write an answer", "an answer")
    } else {
        ("Add a comment", "a comment")
    };
    item(page, Link::Comment, |page, link| {
        page.box_top(vec![Seg::linked(title, Role::Strong, link)], Vec::new());
        let keys = cx.keys;
        let mut press = format!(
            "Press {} to write {what}, or on a comment to reply in its thread",
            keys.comment
        );
        if d.locked {
            press.push_str(" (it's locked: only those with write access can)");
        }
        body(page, vec![Seg::linked(press, Role::Meta, link)]);
        page.box_bottom();
    });
}
