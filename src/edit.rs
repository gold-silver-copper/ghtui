//! Editing what's already on GitHub: your comments and descriptions (in
//! the composer, in place), titles, reactions, and an issue's or pull
//! request's sidebar (labels, assignees, reviewers, milestone) in pickers
//! the way GitHub's sidebar has them. What an action does where you are is
//! decided once, by [`plan`]: the key and the menu both ask it, so they
//! agree, and it asks GitHub's word on what you may do. Each edit goes to
//! GitHub as a [`Change::Edit`], and the page follows GitHub until it
//! shows it (see [`crate::act`]).

use crate::route::{Route, issues, pulls};
use std::collections::BTreeSet;

use crate::api::change::Change;
use crate::api::edit::{Edit, Editable, Named, ReactionContent, Sidebar, Written};
use crate::api::model::{Capped, NodeId, PrRef, RepoId};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::act::{By, Step, Subject};
use crate::compose;
use crate::data::{Data, DataKey};
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::{self, Choice};
use crate::state::{Api, Cmd, Overlay, Remote, Screen, State};
use crate::ui::annotations::{AnnotationKey, ThreadRowKind};
use crate::ui::diff_doc::{Pos, Row};
use crate::ui::overlays::PaletteItem;

/// The actions that edit what's on GitHub, as the menu lists them.
pub const ACTIONS: [Action; 8] = [
    Action::EditComment,
    Action::DeleteComment,
    Action::React,
    Action::EditTitle,
    Action::EditLabels,
    Action::EditAssignees,
    Action::RequestReviews,
    Action::EditMilestone,
];

/// A comment or description being edited in the composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editing {
    pub of: Written,
    pub id: NodeId,
    /// The issue or pull request it's on, which follows GitHub after.
    pub about: Subject,
    /// When it was written: on a page, the composer takes its box's place.
    pub at: String,
    /// "the description", "hubot's comment".
    pub name: String,
}

/// A sidebar field a picker edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Labels,
    Assignees,
    Reviewers,
}

/// What a picker is choosing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choosing {
    /// Labels, assignees or reviewers, by name: what's chosen so far, or
    /// `None` while it's what GitHub has.
    Many {
        field: Field,
        about: Subject,
        chosen: Option<BTreeSet<String>>,
    },
    Milestone {
        about: Subject,
    },
    /// Your reaction to `target` (a comment, or the description).
    React {
        about: Subject,
        target: Editable,
    },
    /// A new title (the picker's field) for `id`, which was `was`.
    Title {
        about: Subject,
        id: NodeId,
        was: String,
    },
}

/// What choosing a picker's row does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// Choose or unchoose this name.
    Toggle(String),
    /// Set the milestone to this one (by name), or none.
    Milestone(Option<String>),
    React(ReactionContent),
    Title(String),
}

impl Choosing {
    pub fn title(&self) -> &'static str {
        match self {
            Choosing::Many {
                field: Field::Labels,
                ..
            } => "Labels",
            Choosing::Many {
                field: Field::Assignees,
                ..
            } => "Assignees",
            Choosing::Many {
                field: Field::Reviewers,
                ..
            } => "Reviewers",
            Choosing::Milestone { .. } => "Milestone",
            Choosing::React { .. } => "React",
            Choosing::Title { .. } => "Edit the title",
        }
    }

    pub fn placeholder(&self) -> &'static str {
        match self {
            Choosing::Many { .. } => "Filter · ↵ checks · ctrl-s applies · esc cancels",
            Choosing::Milestone { .. } => "Filter · ↵ sets it · esc cancels",
            Choosing::React { .. } => "↵ gives or takes back your reaction",
            Choosing::Title { .. } => "The title",
        }
    }

    /// What it reads: the sidebar, for a sidebar field.
    pub fn need(&self) -> Option<DataKey> {
        match self {
            Choosing::Many { about, .. } | Choosing::Milestone { about } => {
                let (repo, number) = numbered(about)?;
                Some(DataKey::Sidebar(repo, number))
            }
            Choosing::React { .. } | Choosing::Title { .. } => None,
        }
    }

    /// Typing filters the rows, except reactions'.
    pub fn filters(&self) -> bool {
        !matches!(self, Choosing::React { .. })
    }
}

/// An issue's or pull request's repository and number.
fn numbered(about: &Subject) -> Option<(RepoId, u64)> {
    match about {
        Subject::Pr(pr) => Some((pr.repo.clone(), pr.number)),
        Subject::Issue(repo, number) => Some((repo.clone(), *number)),
        _ => None,
    }
}

/// A comment or description on screen.
struct Spot {
    of: Written,
    editable: Editable,
    body: String,
    author: String,
    at: String,
    about: Subject,
}

impl Spot {
    fn name(&self) -> String {
        match self.of {
            Written::Issue | Written::Pr => "the description".to_owned(),
            Written::IssueComment | Written::ReviewComment => format!("{}'s comment", self.author),
            Written::Review => format!("{}'s review", self.author),
        }
    }
}

/// The issue or pull request on screen: a page about one (any of a pull
/// request's tabs) or a pull request's files.
fn subject_here(state: &State) -> Option<Subject> {
    match state.screen() {
        Screen::Page(p) => match &p.route {
            Route::Issues(issues::Issue { repo, number }) => {
                Some(Subject::Issue(repo.clone(), *number))
            }
            Route::Pulls(pulls::Pr { pr, .. }) => Some(Subject::Pr(pr.clone())),
            _ => None,
        },
        Screen::Diff(d) => d.of.pr().cloned().map(Subject::Pr),
    }
}

/// The issue on screen, loaded.
fn issue<'a>(
    state: &'a State,
    repo: &RepoId,
    number: u64,
) -> Option<&'a crate::api::browse::IssueDetail> {
    match state.get(&DataKey::Issue(repo.clone(), number)) {
        Some(Data::Issue(Some(issue))) => Some(issue),
        _ => None,
    }
}

fn pr_detail<'a>(state: &'a State, pr: &PrRef) -> Option<&'a crate::api::model::PrDetail> {
    state.pr(pr)
}

/// The description of the issue or pull request on screen, as a spot.
fn description(state: &State, about: &Subject) -> Result<Spot, String> {
    let loading = || "It's still loading".to_owned();
    match about {
        Subject::Issue(repo, number) => {
            let d = issue(state, repo, *number).ok_or_else(loading)?;
            Ok(Spot {
                of: Written::Issue,
                editable: d.editable.clone(),
                body: d.body.clone(),
                author: d.author.clone(),
                at: d.created_at.clone(),
                about: about.clone(),
            })
        }
        Subject::Pr(pr) => {
            let d = pr_detail(state, pr).ok_or_else(loading)?;
            Ok(Spot {
                of: Written::Pr,
                editable: d.editable.clone(),
                body: d.body.clone(),
                author: d.author.clone(),
                at: d.created_at.clone(),
                about: about.clone(),
            })
        }
        _ => Err(loading()),
    }
}

/// The comment selected on a page (its description is one), or, with
/// `fallback`, the description when no comment is.
fn page_spot(state: &State, about: &Subject, fallback: bool) -> Result<Spot, String> {
    let Screen::Page(p) = state.screen() else {
        return Err(String::new());
    };
    let Some(crate::ui::page::Link::Quote { author, at, .. }) = p.selected_link() else {
        if fallback {
            return description(state, about);
        }
        let next = state.first_key(Action::NextItem);
        return Err(format!(
            "Select a comment first: {next} goes comment by comment"
        ));
    };
    let said = description(state, about)?;
    if said.at == *at && said.author == *author {
        return Ok(said);
    }
    let timeline = match about {
        Subject::Issue(repo, number) => issue(state, repo, *number).map(|d| &d.timeline),
        Subject::Pr(pr) => state.activity(pr).map(|a| &a.timeline),
        _ => None,
    };
    let timeline = timeline.map_or(&[][..], |t| &t.items[..]);
    // A review's summary is written like a comment.
    let review = timeline.iter().find_map(|i| match i {
        crate::api::timeline::Item::Review(r) if r.at == *at && r.author == *author => Some(r),
        _ => None,
    });
    if let Some(r) = review {
        return Ok(Spot {
            of: Written::Review,
            editable: r.editable.clone(),
            body: r.body.clone(),
            author: r.author.clone(),
            at: r.at.clone(),
            about: about.clone(),
        });
    }
    let found = (timeline.iter())
        .find_map(|i| match i {
            crate::api::timeline::Item::Comment(c)
                if c.created_at == *at && c.author == *author =>
            {
                Some(c)
            }
            _ => None,
        })
        .and_then(|c| Some((c, c.editable.clone()?)));
    match found {
        Some((c, editable)) => Ok(Spot {
            of: Written::IssueComment,
            editable,
            body: c.body.clone(),
            author: c.author.clone(),
            at: c.created_at.clone(),
            about: about.clone(),
        }),
        None => Err("That's a review's summary: ghtui edits comments and descriptions".into()),
    }
}

/// The review comment under the diff's cursor.
fn diff_spot(state: &State, pr: &PrRef) -> Result<Spot, String> {
    let (Screen::Diff(screen), Some(diff)) = (state.screen(), state.diff()) else {
        return Err("The diff hasn't loaded yet".into());
    };
    let doc = &diff.doc;
    let cursor = screen.cursor;
    let move_to = "Move to a comment in a thread";
    let ann = doc.annotation_under(cursor, screen.half).ok_or(move_to)?;
    let annotation = doc.annotations().get(ann as usize).ok_or(move_to)?;
    if let AnnotationKey::Draft(_) = annotation.key {
        let (open, delete) = (
            state.first_key(Action::Open),
            state.first_key(Action::Delete),
        );
        return Err(format!(
            "That's your draft: {open} edits it, {delete} deletes it"
        ));
    }
    // The comment whose rows the cursor is on: back to its author's line.
    let file = doc.files().get(cursor.file).ok_or(move_to)?;
    let mut comment = None;
    for row in (0..=cursor.row).rev() {
        let Some(Row::Thread(t)) = doc.row(Pos { row, ..cursor }) else {
            break;
        };
        let Some(r) = file.thread_row(t).filter(|r| r.ann == ann) else {
            break;
        };
        match r.kind {
            ThreadRowKind::Head { comment: c, .. } => {
                comment = Some(c);
                break;
            }
            ThreadRowKind::Body | ThreadRowKind::Reactions { .. } => {}
            _ => break,
        }
    }
    let c = comment
        .and_then(|c| annotation.comments.get(c))
        .ok_or(move_to)?;
    let editable = c.editable.clone().ok_or(move_to)?;
    Ok(Spot {
        of: Written::ReviewComment,
        editable,
        body: c.body.clone(),
        author: c.author.clone(),
        at: String::new(),
        about: Subject::Pr(pr.clone()),
    })
}

/// The comment or description an action is about here.
fn spot(state: &State, about: &Subject, fallback: bool) -> Result<Spot, String> {
    match (state.screen(), about) {
        (Screen::Diff(_), Subject::Pr(pr)) => diff_spot(state, pr),
        _ => page_spot(state, about, fallback),
    }
}

/// Where `action` works, for when it doesn't here.
fn elsewhere(action: Action) -> String {
    match action {
        Action::EditComment | Action::DeleteComment | Action::React => {
            "That works on a comment, an issue or a pull request".into()
        }
        _ => "That works on an issue or a pull request".into(),
    }
}

fn plan(state: &State, action: Action) -> Result<Step, String> {
    let about = subject_here(state).ok_or_else(|| elsewhere(action))?;
    let diff = matches!(state.screen(), Screen::Diff(_));
    match action {
        Action::EditComment => {
            let spot = spot(state, &about, true)?;
            if !spot.editable.can_update {
                return Err(format!(
                    "GitHub doesn't let you edit {}: it takes its author or write access",
                    spot.name()
                ));
            }
            let editing = Editing {
                of: spot.of,
                id: spot.editable.id.clone(),
                about: spot.about.clone(),
                at: spot.at.clone(),
                name: spot.name(),
            };
            Ok(Step::Compose(compose::Target::Edit(editing), spot.body))
        }
        Action::DeleteComment => {
            let spot = spot(state, &about, false)?;
            if matches!(spot.of, Written::Issue | Written::Pr) {
                return Err("A description can't be deleted, only edited".into());
            }
            if !spot.editable.can_delete {
                return Err(format!(
                    "GitHub doesn't let you delete {}: it takes its author or write access",
                    spot.name()
                ));
            }
            // Its first line, so it's clear which.
            let first = spot.body.lines().find(|l| !l.trim().is_empty());
            let mut facts = Vec::new();
            if let Some(line) = first {
                let line = crate::ui::text::truncate(line.trim(), 60);
                facts.push((format!("“{line}”"), false));
            }
            facts.push(("Deleting it can't be undone".to_owned(), true));
            let title = format!("Delete {}?", spot.name());
            let delete = Change::Edit(Edit::Delete {
                of: spot.of,
                id: spot.editable.id.clone(),
            });
            Ok(Step::Ask(Box::new(crate::act::confirm(
                action,
                spot.about,
                title,
                facts,
                vec![("Delete".into(), delete)],
            ))))
        }
        Action::React => {
            let spot = spot(state, &about, true)?;
            if !spot.editable.can_react {
                return Err(format!("GitHub doesn't let you react to {}", spot.name()));
            }
            choose(Choosing::React {
                about: spot.about,
                target: spot.editable,
            })
        }
        _ if diff => Err(format!("{}: on the conversation", action.description())),
        Action::EditTitle => {
            let spot = description(state, &about)?;
            if !spot.editable.can_update {
                return Err(
                    "GitHub doesn't let you edit its title: it takes its author or triage access"
                        .into(),
                );
            }
            let was = match &about {
                Subject::Issue(repo, number) => {
                    issue(state, repo, *number).map(|d| d.title.clone())
                }
                Subject::Pr(pr) => pr_detail(state, pr).map(|d| d.title.clone()),
                _ => None,
            };
            choose(Choosing::Title {
                about,
                id: spot.editable.id,
                was: was.unwrap_or_default(),
            })
        }
        Action::EditLabels
        | Action::EditAssignees
        | Action::RequestReviews
        | Action::EditMilestone => {
            let (may, open) = match &about {
                Subject::Issue(repo, number) => {
                    let d = issue(state, repo, *number).ok_or("It's still loading")?;
                    (d.sidebar, true)
                }
                Subject::Pr(pr) => {
                    let d = pr_detail(state, pr).ok_or("It's still loading")?;
                    (d.sidebar, d.state.is_open())
                }
                _ => return Err(elsewhere(action)),
            };
            let triage = "it takes triage access";
            match action {
                Action::EditLabels if !may.labels => {
                    Err(format!("GitHub doesn't let you label it: {triage}"))
                }
                Action::EditAssignees if !may.assignees => {
                    Err(format!("GitHub doesn't let you assign it: {triage}"))
                }
                Action::EditMilestone if !may.milestone => Err(format!(
                    "GitHub doesn't let you set its milestone: {triage}"
                )),
                Action::RequestReviews if !matches!(about, Subject::Pr(_)) => {
                    Err("Reviews are asked for on pull requests".into())
                }
                Action::RequestReviews if !open => {
                    Err("It isn't open: reviews are asked for on open pull requests".into())
                }
                Action::RequestReviews if !may.reviewers => {
                    Err(format!("GitHub doesn't let you ask for reviews: {triage}"))
                }
                Action::EditMilestone => choose(Choosing::Milestone { about }),
                _ => {
                    let field = match action {
                        Action::EditLabels => Field::Labels,
                        Action::EditAssignees => Field::Assignees,
                        _ => Field::Reviewers,
                    };
                    choose(Choosing::Many {
                        field,
                        about,
                        chosen: None,
                    })
                }
            }
        }
        _ => Err(elsewhere(action)),
    }
}

/// What editing issues and pull requests adds: its own actions.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &ACTIONS,
    act,
    offers: |state, out| out.extend(doables(state)),
    ..crate::features::Feature::NONE
};

/// Runs one of [`ACTIONS`] on what's on screen.
#[must_use]
pub fn act(state: &mut State, action: Action) -> Vec<Cmd> {
    let planned = plan(state, action).and_then(|step| crate::act::settled(state).map(|()| step));
    crate::act::take(state, planned)
}

/// Choosing in the picker.
#[expect(clippy::unnecessary_wraps, reason = "a plan's step")]
fn choose(choosing: Choosing) -> Result<Step, String> {
    Ok(Step::Pick(picker::Kind::Edit(choosing)))
}

/// The edits the menu offers here: each action, its label, and why it
/// can't be made now, if it can't.
pub fn doables(state: &State) -> Vec<Doable> {
    let Some(about) = subject_here(state) else {
        return Vec::new();
    };
    let diff = matches!(state.screen(), Screen::Diff(_));
    let what = spot(state, &about, true).map_or_else(|_| "the comment".to_owned(), |s| s.name());
    ACTIONS
        .into_iter()
        .filter(|a| {
            !diff
                || matches!(
                    a,
                    Action::EditComment | Action::DeleteComment | Action::React
                )
        })
        .filter(|a| *a != Action::RequestReviews || matches!(about, Subject::Pr(_)))
        // A draft's are your drafts' ([`crate::review::takes`]).
        .filter(|a| !crate::review::takes(state, *a))
        .map(|action| {
            let label = match action {
                Action::EditComment => format!("Edit {what}"),
                Action::DeleteComment if what != "the description" => format!("Delete {what}"),
                Action::React => format!("React to {what}"),
                Action::EditTitle => "Edit the title".into(),
                Action::EditLabels => "Labels".into(),
                Action::EditAssignees => "Assignees".into(),
                Action::RequestReviews => "Reviewers".into(),
                Action::EditMilestone => "Milestone".into(),
                _ => action.description().into(),
            };
            crate::nav::row("Edit", action, label, "", plan(state, action).err())
        })
        .collect()
}

// ---- pickers --------------------------------------------------------------------

use crate::picker::Rows;

fn row(
    label: impl Into<String>,
    hint: impl Into<String>,
    pick: Option<Pick>,
) -> (PaletteItem, Option<Choice>) {
    let item = crate::picker::item(label, hint);
    (item, pick.map(Choice::Edit))
}

/// The sidebar a picker reads, or the row saying it's loading or why it
/// couldn't.
fn sidebar<'a>(state: &'a State, c: &Choosing) -> Result<&'a Sidebar, Rows> {
    let Some(key) = c.need() else {
        return Err(Vec::new());
    };
    #[expect(clippy::disallowed_methods, reason = "said as the picker's row")]
    let listed = Remote::fetched(state.remote(&key), "")
        .pick(<Sidebar as crate::data::Picked>::pick)
        .text("the sidebar");
    listed.map_err(|why| vec![row(why, "", None)])
}

/// The teams a pull request may ask, as listed.
fn teams(s: &Sidebar) -> impl Iterator<Item = &Named> {
    s.teams.iter().flatten().flat_map(|t| t.iter())
}

/// What's set now, by name, and what can be.
fn field_names(s: &Sidebar, field: Field) -> (Vec<&Named>, Vec<&Named>) {
    let r = &s.requested;
    match field {
        Field::Labels => (s.labels.iter().collect(), s.all_labels.iter().collect()),
        Field::Assignees => (s.assignees.iter().collect(), s.users.iter().collect()),
        Field::Reviewers => {
            let set = (r.users.iter()).chain(&r.teams).chain(&r.bots).collect();
            // GitHub doesn't ask an author to review their own.
            let users = (s.users.iter()).filter(|u| s.author.as_deref() != Some(u.name.as_str()));
            (set, users.chain(teams(s)).collect())
        }
    }
}

/// The picker's rows: what's set first, then the rest, as `q` filters
/// them.
pub fn rows(state: &State, c: &Choosing, q: &str) -> Rows {
    let fits = |name: &str| picker::fuzzy_score(q, name).is_some();
    match c {
        Choosing::React { target, .. } => ReactionContent::ALL
            .into_iter()
            .map(|content| {
                let n = (target.reactions.iter())
                    .find(|r| r.content == content)
                    .map_or(0, |r| r.count);
                let mine = target.reacted(content);
                let mark = if mine { "✓" } else { " " };
                let hint = match (mine, n) {
                    (true, n) => format!("yours · {n}"),
                    (false, 0) => String::new(),
                    (false, n) => n.to_string(),
                };
                let label = format!("{mark} {} {}", content.emoji(), content.label());
                row(label, hint, Some(Pick::React(content)))
            })
            .collect(),
        Choosing::Title { was, .. } => {
            if q.is_empty() {
                vec![row("Type a title", "", None)]
            } else if q == was.trim() {
                vec![row("That's its title now", "", None)]
            } else {
                vec![row(
                    format!("Rename it “{q}”"),
                    "↵",
                    Some(Pick::Title(q.to_owned())),
                )]
            }
        }
        Choosing::Milestone { .. } => {
            let s = match sidebar(state, c) {
                Ok(s) => s,
                Err(rows) => return rows,
            };
            let now = s.milestone.as_ref().map(|m| m.name.as_str());
            let mark = |on: bool| if on { "✓" } else { " " };
            let mut rows = Vec::new();
            if fits("No milestone") {
                rows.push(row(
                    format!("{} No milestone", mark(now.is_none())),
                    "",
                    Some(Pick::Milestone(None)),
                ));
            }
            let mut names: Vec<&str> = now.into_iter().collect();
            names.extend(
                s.milestones
                    .iter()
                    .map(|m| m.name.as_str())
                    .filter(|m| Some(*m) != now),
            );
            for name in names.into_iter().filter(|n| fits(n)) {
                let hint = if Some(name) == now { "current" } else { "" };
                rows.push(row(
                    format!("{} {name}", mark(Some(name) == now)),
                    hint,
                    Some(Pick::Milestone(Some(name.to_owned()))),
                ));
            }
            more(
                &mut rows,
                s.milestones.left_out(),
                "milestone",
                "milestones",
            );
            rows
        }
        Choosing::Many { field, chosen, .. } => {
            let s = match sidebar(state, c) {
                Ok(s) => s,
                Err(rows) => return rows,
            };
            let (set, options) = field_names(s, *field);
            let set: Vec<&str> = set.iter().map(|n| n.name.as_str()).collect();
            let chosen: BTreeSet<&str> = match chosen {
                Some(chosen) => chosen.iter().map(String::as_str).collect(),
                None => set.iter().copied().collect(),
            };
            let mut names: Vec<&str> = set.clone();
            names.extend(
                options
                    .iter()
                    .map(|n| n.name.as_str())
                    .filter(|n| !set.contains(n)),
            );
            let mut rows: Rows = names
                .into_iter()
                .filter(|n| fits(n))
                .map(|name| {
                    let (on, was) = (chosen.contains(name), set.contains(&name));
                    let hint = match (on, was) {
                        (true, false) => "adding",
                        (false, true) => "removing",
                        _ => "",
                    };
                    let mark = if on { "✓" } else { " " };
                    row(
                        format!("{mark} {name}"),
                        hint,
                        Some(Pick::Toggle(name.to_owned())),
                    )
                })
                .collect();
            let (left_out, one, many) = match field {
                Field::Labels => (s.all_labels.left_out(), "label", "labels"),
                Field::Assignees => (s.users.left_out(), "person", "people"),
                Field::Reviewers => {
                    let teams: u64 = s.teams.iter().flatten().map(Capped::left_out).sum();
                    let left_out = s.users.left_out().saturating_add(teams);
                    (left_out, "person or team", "people and teams")
                }
            };
            more(&mut rows, left_out, one, many);
            if *field == Field::Reviewers
                && let Some(Err(why)) = &s.teams
            {
                rows.push(row(format!("Teams aren't listed: {why}"), "", None));
            }
            if rows.is_empty() {
                rows.push(row("Nothing matches", "", None));
            }
            rows
        }
    }
}

/// A row saying how many more GitHub has than were listed.
fn more(rows: &mut Rows, left_out: u64, one: &str, many: &str) {
    if left_out > 0 {
        let what = if left_out == 1 { one } else { many };
        rows.push(row(
            format!("{left_out} more {what} on GitHub, not listed"),
            "",
            None,
        ));
    }
}

/// Keys in an edit's picker: up and down are the picker's; ↵ picks (or,
/// in a list of many, checks or unchecks); ctrl-s applies what's checked.
#[must_use]
pub fn on_picker_key(state: &mut State, key: KeyEvent, rows: Rows) -> Vec<Cmd> {
    // What's set now: where checking starts.
    let set: Option<BTreeSet<String>> = match &state.overlay {
        Some(Overlay::Picker(p)) => match &p.kind {
            picker::Kind::Edit(
                c @ Choosing::Many {
                    field,
                    chosen: None,
                    ..
                },
            ) => sidebar(state, c)
                .ok()
                .map(|s| names(&field_names(s, *field).0)),
            _ => None,
        },
        _ => None,
    };
    let Some(Overlay::Picker(p)) = &mut state.overlay else {
        return Vec::new();
    };
    let picker::Kind::Edit(c) = &mut p.kind else {
        return Vec::new();
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => state.overlay = None,
        KeyCode::Char('s') if ctrl => return apply(state),
        KeyCode::Enter => {
            let Some((_, Some(Choice::Edit(pick)))) = rows.into_iter().nth(p.selected) else {
                return Vec::new();
            };
            if let (Pick::Toggle(name), Choosing::Many { chosen, .. }) = (&pick, c) {
                let chosen = chosen.get_or_insert_with(|| set.unwrap_or_default());
                if !chosen.remove(name) {
                    chosen.insert(name.clone());
                }
                return Vec::new();
            }
            return send_pick(state, pick);
        }
        _ if c.filters() => {
            crate::nav::type_into(&mut p.input, key);
            p.selected = 0;
        }
        // Reactions aren't filtered: j and k move, as in a list.
        KeyCode::Char('j') => p.selected = (p.selected + 1).min(rows.len().saturating_sub(1)),
        KeyCode::Char('k') => p.selected = p.selected.saturating_sub(1),
        _ => {}
    }
    Vec::new()
}

fn names(list: &[&Named]) -> BTreeSet<String> {
    list.iter().map(|n| n.name.clone()).collect()
}

/// The node ID of the issue or pull request `about`, once it's loaded.
fn node(state: &State, about: &Subject) -> Option<NodeId> {
    match about {
        Subject::Issue(repo, number) => issue(state, repo, *number).map(|d| d.id.clone()),
        Subject::Pr(pr) => pr_detail(state, pr).map(|d| d.id.clone()),
        _ => None,
    }
}

/// Sends what a picker's row picks, closing the picker.
#[must_use]
fn send_pick(state: &mut State, pick: Pick) -> Vec<Cmd> {
    let Some(Overlay::Picker(p)) = &state.overlay else {
        return Vec::new();
    };
    let picker::Kind::Edit(c) = &p.kind else {
        return Vec::new();
    };
    let edit = match (pick, c) {
        (Pick::React(content), Choosing::React { about, target, .. }) => Some((
            about.clone(),
            Edit::React {
                subject: target.id.clone(),
                content,
                on: !target.reacted(content),
            },
        )),
        (Pick::Title(title), Choosing::Title { about, id, .. }) => Some((
            about.clone(),
            Edit::Title {
                pr: matches!(about, Subject::Pr(_)),
                id: id.clone(),
                title,
            },
        )),
        (Pick::Milestone(name), Choosing::Milestone { about }) => {
            let Ok(s) = sidebar(state, c) else {
                return Vec::new();
            };
            if s.milestone.as_ref().map(|m| &m.name) == name.as_ref() {
                state.overlay = None;
                return state.just_say("That's its milestone now");
            }
            let milestone = name.and_then(|name| {
                (s.milestones.iter())
                    .chain(&s.milestone)
                    .find(|m| m.name == name)
                    .cloned()
            });
            node(state, about).map(|id| {
                let pr = matches!(about, Subject::Pr(_));
                (about.clone(), Edit::Milestone { pr, id, milestone })
            })
        }
        _ => None,
    };
    let Some((about, edit)) = edit else {
        return Vec::new();
    };
    send(state, about, edit)
}

/// Sends `edit`, closing the picker and saying what's happening.
#[must_use]
fn send(state: &mut State, about: Subject, edit: Edit) -> Vec<Cmd> {
    state.overlay = None;
    state.info(format!("{}…", doing(&edit)));
    vec![crate::act::send(Change::Edit(edit), By::Edit(about))]
}

/// ctrl-s in a picker of many: sends what changed from what's set.
#[must_use]
fn apply(state: &mut State) -> Vec<Cmd> {
    let Some(Overlay::Picker(p)) = &state.overlay else {
        return Vec::new();
    };
    let picker::Kind::Edit(
        c @ Choosing::Many {
            field,
            about,
            chosen,
        },
    ) = &p.kind
    else {
        return Vec::new();
    };
    let (Ok(s), Some(subject)) = (sidebar(state, c), node(state, about)) else {
        return Vec::new();
    };
    let (set, options) = field_names(s, *field);
    let was = names(&set);
    let now = chosen.clone().unwrap_or_else(|| was.clone());
    if now == was {
        state.overlay = None;
        return state.just_say("Nothing changed");
    }
    // Each name once, what's set first.
    let mut all: Vec<&Named> = set.clone();
    all.extend(options.iter().filter(|n| !set.contains(n)));
    let named = |names: Vec<&String>| -> Vec<Named> {
        (names.into_iter())
            .filter_map(|name| all.iter().find(|n| n.name == *name).map(|n| (*n).clone()))
            .collect()
    };
    let add = named(now.difference(&was).collect());
    let remove = named(was.difference(&now).collect());
    let edit = match field {
        Field::Labels => Edit::Labels {
            subject,
            add,
            remove,
        },
        Field::Assignees => Edit::Assignees {
            subject,
            add,
            remove,
        },
        Field::Reviewers => {
            if s.requested.left_out > 0 {
                return state
                    .just_say("GitHub listed only some of its reviewers: change them on GitHub");
            }
            // Asking replaces who's asked: everyone chosen, by kind.
            let r = &s.requested;
            let listed: Vec<&Named> = teams(s).collect();
            let (teams, rest): (Vec<Named>, Vec<Named>) = named(now.iter().collect())
                .into_iter()
                .partition(|n| r.teams.contains(n) || listed.contains(&n));
            let (bots, users) = rest.into_iter().partition(|n| r.bots.contains(n));
            Edit::Reviewers {
                pr: subject,
                users,
                teams,
                bots,
            }
        }
    };
    let about = about.clone();
    send(state, about, edit)
}

// ---- following GitHub -----------------------------------------------------------

/// The description, comments and reviews' summaries on the issue or pull
/// request, each with its text; `None` until they're loaded.
fn said<'a>(state: &'a State, about: &Subject) -> Option<Vec<(&'a Editable, &'a str)>> {
    let (description, body, timeline) = match about {
        Subject::Issue(repo, number) => {
            let d = issue(state, repo, *number)?;
            (&d.editable, &d.body, &d.timeline)
        }
        Subject::Pr(pr) => {
            let d = pr_detail(state, pr)?;
            (&d.editable, &d.body, &state.activity(pr)?.timeline)
        }
        _ => return None,
    };
    let written = timeline.iter().filter_map(|i| match i {
        crate::api::timeline::Item::Comment(c) => Some((c.editable.as_ref()?, c.body.as_str())),
        crate::api::timeline::Item::Review(r) => Some((&r.editable, r.body.as_str())),
        crate::api::timeline::Item::Event(_) => None,
    });
    Some(
        std::iter::once((description, body.as_str()))
            .chain(written)
            .collect(),
    )
}

/// The same text, but for line endings and the ends' whitespace.
fn same(a: &str, b: &str) -> bool {
    a.replace("\r\n", "\n").trim() == b.replace("\r\n", "\n").trim()
}

/// Whether the names in `list` have every one of `want` and none of
/// `gone`.
fn has(list: &[String], want: &[Named], gone: &[Named]) -> bool {
    want.iter().all(|n| list.contains(&n.name)) && !gone.iter().any(|n| list.contains(&n.name))
}

/// Whether the issue or pull request `about` shows `edit`. A review
/// comment shows in the diff's threads, which are fetched again after.
pub fn shown(state: &State, edit: &Edit, about: &Subject) -> bool {
    // `Some(None)`: everything's loaded, and it isn't there.
    let find =
        |id: &NodeId| said(state, about).map(|all| all.into_iter().find(|(e, _)| e.id == *id));
    let (title, labels, assignees, milestone) = match about {
        Subject::Issue(repo, number) => match issue(state, repo, *number) {
            Some(d) => (&d.title, &d.labels, &d.assignees, d.milestone.as_ref()),
            None => return false,
        },
        Subject::Pr(pr) => match pr_detail(state, pr) {
            Some(d) => (&d.title, &d.labels, &d.assignees, d.milestone.as_ref()),
            None => return false,
        },
        _ => return true,
    };
    match edit {
        Edit::Body {
            of: Written::ReviewComment,
            ..
        }
        | Edit::Delete {
            of: Written::ReviewComment,
            ..
        } => true,
        Edit::Body { id, body, .. } => match find(id) {
            Some(Some((_, now))) => same(now, body),
            Some(None) => true,
            None => false,
        },
        Edit::Delete { id, .. } => matches!(find(id), Some(None)),
        Edit::React {
            subject,
            content,
            on,
        } => match find(subject) {
            Some(Some((e, _))) => e.reacted(*content) == *on,
            Some(None) => true,
            None => false,
        },
        Edit::Title { title: new, .. } => same(title, new),
        Edit::Labels { add, remove, .. } => {
            let names: Vec<String> = labels.iter().map(|l| l.name.clone()).collect();
            has(&names, add, remove)
        }
        Edit::Assignees { add, remove, .. } => has(assignees, add, remove),
        Edit::Milestone {
            milestone: want, ..
        } => milestone.map(|m| m.title.as_str()) == want.as_ref().map(|m| m.name.as_str()),
        Edit::Reviewers {
            users, teams, bots, ..
        } => {
            let Subject::Pr(pr) = about else {
                return true;
            };
            let Some(a) = state.activity(pr) else {
                return false;
            };
            let asked: BTreeSet<&str> = (a.requested.iter())
                .map(|r| match &r.who {
                    crate::api::browse::Reviewer::User(n)
                    | crate::api::browse::Reviewer::Team(n) => n.as_str(),
                })
                .collect();
            let wanted: BTreeSet<&str> = (users.iter().chain(teams).chain(bots))
                .map(|n| n.name.as_str())
                .collect();
            asked == wanted
        }
    }
}

/// What else the page fetches again while it waits for an edit: a pull
/// request's comments and reviewers are its conversation's.
pub fn polls(about: &Subject) -> Option<DataKey> {
    match about {
        Subject::Pr(pr) => Some(DataKey::PrActivity(pr.clone())),
        _ => None,
    }
}

/// After an edit made in a pull request's files: its threads, fetched
/// again to show it.
pub fn after(state: &State) -> Option<Cmd> {
    match state.screen() {
        Screen::Diff(d) => (d.of.pr()).map(|pr| Cmd::Api(Api::FetchThreads(pr.clone()))),
        Screen::Page(_) => None,
    }
}

/// What an edit is doing while it's sent.
pub fn doing(edit: &Edit) -> &'static str {
    match edit {
        Edit::Body { .. } => "Saving",
        Edit::Delete { .. } => "Deleting it",
        Edit::Title { .. } => "Renaming it",
        Edit::React { on: true, .. } => "Reacting",
        Edit::React { .. } => "Taking back your reaction",
        Edit::Labels { .. } => "Changing the labels",
        Edit::Assignees { .. } => "Changing the assignees",
        Edit::Reviewers { .. } => "Asking for reviews",
        Edit::Milestone { .. } => "Setting the milestone",
    }
}

/// What an edit did, once it's accepted.
pub fn done(edit: &Edit) -> &'static str {
    match edit {
        Edit::Body { .. } => "Saved",
        Edit::Delete { .. } => "Deleted",
        Edit::Title { .. } => "Renamed",
        Edit::React { on: true, .. } => "Reacted",
        Edit::React { .. } => "Reaction taken back",
        Edit::Labels { .. } => "Labels changed",
        Edit::Assignees { .. } => "Assignees changed",
        Edit::Reviewers { .. } => "Reviewers asked",
        Edit::Milestone {
            milestone: None, ..
        } => "Milestone cleared",
        Edit::Milestone { .. } => "Milestone set",
    }
}

/// What was refused, for "… refused it".
pub fn what(edit: &Edit) -> &'static str {
    match edit {
        Edit::Body { .. } => "the edit",
        Edit::Delete { .. } => "deleting it",
        Edit::Title { .. } => "the new title",
        Edit::React { .. } => "the reaction",
        Edit::Labels { .. } => "the labels",
        Edit::Assignees { .. } => "the assignees",
        Edit::Reviewers { .. } => "the review request",
        Edit::Milestone { .. } => "the milestone",
    }
}
