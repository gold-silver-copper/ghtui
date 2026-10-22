//! An organization's people as its People tab lists them (with their
//! roles, and for its owners the invitations waiting), and the
//! invitations sent to you: in your notifications, and on their own
//! pages ([`crate::org`], [`crate::invited`]).

use super::*;
use crate::api::org::{Invitations, Member, OrgAccess, OrgRole};
use crate::route::people::Invite;
use crate::ui::pages::SettingLink;

/// An organization's members, with their roles; for its owners, the
/// invitations waiting, and `hint`, the keys that change them.
pub fn members(
    page: &mut Page,
    list: Fetched<'_, Results<Member>>,
    access: Option<&OrgAccess>,
    hint: &str,
    now: u64,
) {
    let admin = access.is_some_and(|a| a.admin);
    if let Some(lacks) = access.and_then(|a| a.lacks.as_ref()) {
        say(page, lacks.clone(), Role::Meta);
        page.blank();
    }
    let Some(l) = list.show(page, "people") else { return };
    let right = if admin {
        vec![button(
            page,
            "Invite member",
            Link::Setting(SettingLink::Add),
        )]
    } else {
        Vec::new()
    };
    list_box(
        page,
        counted("People", l),
        right,
        l,
        "Nobody here yet.",
        |page, m| {
            item(page, url::user(&m.login), |page, link| {
                let mut segs = vec![Seg::linked(m.login.clone(), Role::Link, link)];
                if let Some(name) = &m.name {
                    segs.push(strong(format!("  {name}")));
                }
                let right = match m.role {
                    Some(OrgRole::Owner) => vec![chip("Owner", Bg::PrimaryContainer)],
                    Some(OrgRole::Member) => vec![chip("Member", Bg::SecondaryContainer)],
                    None => Vec::new(),
                };
                page.box_line(segs, right, 0);
            });
        },
    );
    let Some(invitations) = access.and_then(|a| a.invitations.as_ref()) else {
        return;
    };
    page.blank();
    match invitations {
        Err(why) => say(page, why.clone(), Role::Meta),
        Ok(invitations) => {
            let (name, none) = ("Pending invitations", "No invitations waiting.");
            counted_box(page, name, Vec::new(), invitations, none, |page, i| {
                let to = Link::Setting(SettingLink::OrgInvitation(i.id, i.login.clone()));
                item(page, to, |page, link| {
                    let when = if i.failed {
                        "couldn't be delivered".to_owned()
                    } else {
                        format!("invited {}", time::ago_iso(&i.created_at, now))
                    };
                    let by = i
                        .inviter
                        .as_ref()
                        .map_or_else(String::new, |l| format!(" by @{l}"));
                    let right = vec![
                        meta(format!("{when}{by}  ")),
                        chip(i.role.clone(), Bg::SecondaryContainer),
                    ];
                    let who = Seg::linked(i.who(), Role::Strong, link);
                    page.box_line(vec![who], right, 0);
                });
            });
        }
    }
    if admin {
        page.blank();
        say(page, hint.to_owned(), Role::Meta);
    }
}

/// What an invitation is to, as a row says it.
fn repo_line(i: &crate::api::org::RepoInvitation) -> String {
    let by = i
        .inviter
        .as_deref()
        .map_or_else(|| "You're".to_owned(), |l| format!("@{l}"));
    let verb = if i.inviter.is_some() {
        "invited you"
    } else {
        "invited"
    };
    format!("{by} {verb} to collaborate on {}", i.repo)
}

/// The invitations sent to you, at the top of your notifications: each
/// opens its page.
pub fn invited_box(page: &mut Page, all: &Invitations, now: u64) {
    let orgs = all.orgs.as_ref().map_or(&[][..], Vec::as_slice);
    if all.repos.is_empty() && orgs.is_empty() {
        return;
    }
    let title = vec![strong(format!(
        "Invitations  {}",
        all.repos.len() + orgs.len()
    ))];
    page.box_top(title, Vec::new());
    for i in &all.repos {
        let to = url::repo(&i.repo) + "/invitations";
        item(page, to, |page, link| {
            let mut right = Vec::new();
            if i.expired {
                right.push(meta("expired  "));
            } else {
                right.push(meta(format!("{}  ", time::ago_iso(&i.created_at, now))));
            }
            if let Some(p) = i.permission {
                right.push(chip(p.name(), Bg::SecondaryContainer));
            }
            let text = Seg::linked(repo_line(i), Role::Strong, link);
            page.box_line(vec![text], right, 0);
        });
    }
    for i in orgs {
        let to = format!("{}/orgs/{}/invitation", url::base(), i.org);
        item(page, to, |page, link| {
            let text = format!("@{} invited you to join", i.org);
            let right = vec![chip(i.role.name(), Bg::SecondaryContainer)];
            page.box_line(vec![Seg::linked(text, Role::Strong, link)], right, 0);
        });
    }
    page.box_bottom();
    page.blank();
}

/// One of an invitation's buttons, a row of its own, with the key that
/// answers it too.
fn answer_button(page: &mut Page, text: &str, accept: bool, key: &str) {
    item(page, crate::invited::button(accept), |page, link| {
        let bg = if accept {
            Bg::SuccessContainer
        } else {
            Bg::ContainerHigh
        };
        let button = Seg::linked(format!(" {text} "), Role::Chip(bg), link);
        page.line(vec![button, meta(format!("  {key}"))]);
    });
}

/// An invitation sent to you, as its page on GitHub shows it: what it's
/// to, who sent it, what it gives, and its buttons (`key` answers it).
pub fn invitation(page: &mut Page, invite: &Invite, all: &Invitations, key: &str, now: u64) {
    match invite {
        Invite::Repo(repo) => {
            let Some(i) = all.repos.iter().find(|i| i.repo == *repo) else {
                say(
                    page,
                    format!("No invitation to {repo} is waiting for you."),
                    Role::Body,
                );
                page.blank();
                let link = link_seg(page, repo.to_string(), url::repo(repo), Role::Link);
                page.wrapped(
                    vec![meta("If you accepted it, it's at "), link],
                    0,
                    Frame::None,
                );
                return;
            };
            say(page, repo_line(i), Role::Title);
            let when = format!("Sent {}", time::ago_iso(&i.created_at, now));
            say(page, when, Role::Meta);
            if let Some(p) = i.permission {
                let can = format!("With {} access, you can {}.", p.name(), p.about());
                say(page, can, Role::Body);
            }
            page.blank();
            if i.expired {
                say(
                    page,
                    "It has expired: they can invite you again.",
                    Role::Error,
                );
                return;
            }
            answer_button(page, "Accept invitation", true, key);
            answer_button(page, "Decline", false, key);
        }
        Invite::Org(org) => {
            let orgs = match &all.orgs {
                Ok(orgs) => orgs,
                Err(why) => return say(page, why.clone(), Role::Meta),
            };
            let Some(i) = orgs.iter().find(|i| i.org.eq_ignore_ascii_case(org)) else {
                say(
                    page,
                    format!("No invitation to @{org} is waiting for you."),
                    Role::Body,
                );
                return;
            };
            say(page, format!("@{} invited you to join", i.org), Role::Title);
            let role = i.role.name().to_lowercase();
            say(
                page,
                format!("As its {role}, you {}.", i.role.about()),
                Role::Body,
            );
            page.blank();
            answer_button(page, &format!("Join @{}", i.org), true, key);
            say(
                page,
                "Declining is done on github.com: GitHub's API can't decline an organization's invitation.",
                Role::Meta,
            );
        }
    }
}
