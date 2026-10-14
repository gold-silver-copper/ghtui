//! A repository's milestones: made, edited (title, due date,
//! description), closed or reopened, and deleted.

use crate::api::browse::{MilestoneDetail, MilestoneInfo, MilestoneList};
use crate::api::housekeeping::MilestoneChange;
use crate::api::model::RepoId;
use crate::route::issues;
use crate::ui::page::{Link, Page};
use crate::ui::pages::{Mark, TextBox, WriteFormView};

use super::{
    Chosen, Expected, Field, Form, FormOf, MilestoneWrite, Subject, Work, ask, expect, failed,
    form, form_mut, form_row, item, refused, selected,
};
use crate::data::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::route::Route;
use crate::state::{Cmd, State};
use crate::writes::{Work as Writes, row};

const WHAT: &str = "Changing milestones";

/// A milestone's link: its page.
fn milestone_url(repo: &RepoId, number: u64) -> String {
    format!("{}/milestone/{number}", crate::ui::pages::url::repo(repo))
}

/// The milestone on screen: its own page's, or the one selected in the
/// list.
fn picked(state: &State) -> Option<MilestoneInfo> {
    match state.route()? {
        Route::Issues(issues::Milestone { repo, number }) => state
            .picked::<MilestoneDetail>(&DataKey::Milestone(repo.clone(), *number))
            .map(|d| d.info.clone()),
        Route::Issues(issues::Milestones { repo, closed }) => {
            let list =
                state.picked::<MilestoneList>(&DataKey::Milestones(repo.clone(), *closed))?;
            selected(state, &list.results.items, |m| {
                milestone_url(repo, m.number)
            })
            .cloned()
        }
        _ => None,
    }
}

/// `,o` or `,e`: the milestone's form.
#[must_use]
pub(super) fn open(state: &mut State, repo: &RepoId, number: Option<u64>) -> Vec<Cmd> {
    if let Some(why) = refused(state, repo, WHAT) {
        state.info(why);
        return Vec::new();
    }
    super::open(
        state,
        FormOf::Milestone {
            repo: repo.clone(),
            number,
        },
    )
}

#[must_use]
pub(super) fn edit(state: &mut State, repo: &RepoId) -> Vec<Cmd> {
    let Some(m) = picked(state) else {
        state.info("Select a milestone first");
        return Vec::new();
    };
    open(state, repo, Some(m.number))
}

/// Fills the form from the milestone once it's loaded; a new one starts
/// with its title.
#[must_use]
pub(super) fn follow(
    state: &mut State,
    route: &Route,
    repo: &RepoId,
    number: Option<u64>,
) -> Vec<Cmd> {
    if form(state, route).started {
        return Vec::new();
    }
    let info = match number {
        Some(n) => {
            let key = DataKey::Milestone(repo.clone(), n);
            let Some(d) = state.picked::<MilestoneDetail>(&key) else {
                return Vec::new();
            };
            Some(d.info.clone())
        }
        None => None,
    };
    let f = form_mut(state, route);
    f.started = true;
    if let Some(m) = info {
        f.set(Field::Title, m.title);
        let due = m.due_on.as_deref().and_then(|d| d.get(..10)).unwrap_or("");
        f.set(Field::Due, due);
        f.text = m.description;
    }
    state.data_gen += 1;
    if number.is_none() && state.overlay.is_none() {
        return super::open_field(state, route, Field::Title);
    }
    Vec::new()
}

/// Whether `date` is a day of the calendar, written `YYYY-MM-DD`.
fn valid_date(date: &str) -> bool {
    let parts: Vec<&str> = date.split('-').collect();
    let [y, m, d] = parts.as_slice() else {
        return false;
    };
    let digits = |s: &str, n: usize| s.len() == n && s.chars().all(|c| c.is_ascii_digit());
    if !(digits(y, 4) && digits(m, 2) && digits(d, 2)) {
        return false;
    }
    let (Ok(y), Ok(m), Ok(d)) = (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) else {
        return false;
    };
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

/// The due date picker's rows: the date typed, or none.
pub(super) fn due_rows(
    route: &Route,
    q: &str,
) -> Vec<(
    crate::ui::overlays::PaletteItem,
    Option<crate::writes::Chosen>,
)> {
    let set = |d: &str| Some(Chosen::Field(route.clone(), Field::Due, d.to_owned()));
    let mut rows = Vec::new();
    if !q.is_empty() {
        rows.push(if valid_date(q) {
            item(format!("Due by {q}"), "↵", set(q))
        } else {
            item("A date is YYYY-MM-DD", "", None)
        });
    }
    rows.push(item("No due date", "", set("")));
    rows
}

/// The milestone as the form has it, or why it can't be saved.
pub(super) fn change(f: &Form) -> Result<MilestoneChange, String> {
    let title = f.get(Field::Title).trim();
    if title.is_empty() {
        return Err("A milestone needs a title".into());
    }
    let due = f.get(Field::Due).trim();
    if !due.is_empty() && !valid_date(due) {
        return Err("A due date is YYYY-MM-DD".into());
    }
    Ok(MilestoneChange {
        title: title.to_owned(),
        description: f.text.trim_end().to_owned(),
        due_on: (!due.is_empty()).then(|| due.to_owned()),
    })
}

#[must_use]
pub(super) fn submit(
    state: &mut State,
    route: &Route,
    repo: &RepoId,
    number: Option<u64>,
) -> Vec<Cmd> {
    if let Some(why) = refused(state, repo, WHAT) {
        crate::writes::say(state, why);
        return Vec::new();
    }
    let change = match change(form(state, route)) {
        Ok(c) => c,
        Err(why) => {
            crate::writes::say(state, why);
            return Vec::new();
        }
    };
    form_mut(state, route).sending = true;
    crate::writes::sending(state);
    state.info(format!("Saving the milestone {}…", change.title));
    vec![Cmd::Write(Writes::Housekeeping(Work::Milestone {
        repo: repo.clone(),
        number,
        write: MilestoneWrite::Save(change),
    }))]
}

/// `,x`: closes the milestone, or reopens it. GitHub doesn't ask.
#[must_use]
pub(super) fn toggle(state: &mut State, repo: &RepoId) -> Vec<Cmd> {
    if let Some(why) = refused(state, repo, WHAT) {
        state.info(why);
        return Vec::new();
    }
    let Some(m) = picked(state) else {
        state.info("Select a milestone first");
        return Vec::new();
    };
    let verb = if m.closed { "Reopening" } else { "Closing" };
    state.info(format!("{verb} {}…", m.title));
    vec![Cmd::Write(Writes::Housekeeping(Work::Milestone {
        repo: repo.clone(),
        number: Some(m.number),
        write: MilestoneWrite::Close(!m.closed),
    }))]
}

/// `,D`: asks first, as GitHub does.
#[must_use]
pub(super) fn ask_delete(state: &mut State, repo: &RepoId) -> Vec<Cmd> {
    if let Some(why) = refused(state, repo, WHAT) {
        state.info(why);
        return Vec::new();
    }
    let Some(m) = picked(state) else {
        state.info("Select a milestone first");
        return Vec::new();
    };
    let all = m.open + m.done;
    let on = match all {
        0 => "Nothing is in it".to_owned(),
        1 => "Its issue or pull request stays, without a milestone".to_owned(),
        n => format!("Its {n} issues and pull requests stay, without a milestone"),
    };
    ask(
        state,
        Some(Subject::Milestone(repo.clone(), m.number)),
        format!("Delete the milestone {}?", m.title),
        vec![(on, false), ("It can't be brought back".into(), true)],
        vec![("Delete milestone".into(), super::Answer::Delete)],
    );
    Vec::new()
}

#[must_use]
pub(super) fn delete(state: &mut State, repo: RepoId, number: u64) -> Vec<Cmd> {
    crate::writes::sending(state);
    vec![Cmd::Write(Writes::Housekeeping(Work::Milestone {
        repo,
        number: Some(number),
        write: MilestoneWrite::Delete,
    }))]
}

/// GitHub's answer: back on the list or the milestone's page, which
/// follows GitHub until it shows the change.
#[must_use]
pub(super) fn on_done(
    state: &mut State,
    work: Work,
    result: Result<Option<u64>, String>,
) -> Vec<Cmd> {
    let Work::Milestone {
        repo,
        number,
        write,
    } = work
    else {
        return Vec::new();
    };
    let form = Route::Housekeeping(FormOf::Milestone {
        repo: repo.clone(),
        number,
    });
    let made = number.is_none();
    let number = match result {
        Ok(Some(n)) => n,
        Ok(None) => return Vec::new(),
        Err(why) => {
            if matches!(write, MilestoneWrite::Close(_)) {
                state.error(why);
            } else {
                failed(state, &form, why);
            }
            return Vec::new();
        }
    };
    let asked = state.route().cloned();
    if let Some(route) = &asked {
        crate::writes::close_sender(state, route);
    }
    state.writing.housekeeping.forms.remove(&form);
    let mut cmds = Vec::new();
    if state.route() == Some(&form) {
        cmds.extend(state.back());
        state.forward.clear();
    }
    let (closed, title) = match &write {
        MilestoneWrite::Save(m) => {
            let verb = if made { "Created" } else { "Saved" };
            state.info(format!("{verb} the milestone {}", m.title));
            (Some(false), Some(m.title.clone()))
        }
        MilestoneWrite::Close(closed) => {
            state.info(if *closed {
                "Closed the milestone"
            } else {
                "Reopened the milestone"
            });
            (Some(*closed), None)
        }
        MilestoneWrite::Delete => {
            state.info("Deleted the milestone");
            (None, None)
        }
    };
    // A deleted milestone's page is gone: its list instead.
    if write == MilestoneWrite::Delete
        && let Some(Route::Issues(issues::Milestone { repo: r, number: n })) =
            state.route().cloned()
        && r == repo
        && n == number
    {
        let list = Route::Issues(issues::Milestones {
            repo: repo.clone(),
            closed: false,
        });
        cmds.extend(state.replace(list, true, None));
    }
    // An edit keeps it open or closed as it was: its title is what's
    // waited for, on the list it's on.
    let closed = match (&write, closed) {
        (MilestoneWrite::Save(_), _) => match state.route() {
            Some(Route::Issues(issues::Milestones { closed, .. })) => Some(*closed),
            Some(Route::Issues(issues::Milestone { repo: r, number: n }))
                if *r == repo && *n == number =>
            {
                state
                    .picked::<MilestoneDetail>(&DataKey::Milestone(repo.clone(), number))
                    .map(|d| d.info.closed)
            }
            _ => Some(false),
        },
        (_, closed) => closed,
    };
    let list = match state.route() {
        Some(Route::Issues(issues::Milestones { repo: r, closed })) if *r == repo => {
            Some(Some(*closed))
        }
        Some(Route::Issues(issues::Milestone { repo: r, number: n }))
            if *r == repo && *n == number =>
        {
            Some(None)
        }
        _ => None,
    };
    if let (Some(list), Some(page)) = (list, state.route().cloned()) {
        expect(
            state,
            page.clone(),
            Expected::Milestone {
                repo,
                number,
                list,
                closed,
                title,
            },
        );
        cmds.extend(state.ensure_route(&page, true));
    }
    cmds
}

/// The menu's rows on the milestones and a milestone's page.
pub(super) fn doables(state: &State, out: &mut Vec<Doable>) {
    let repo = state.route().and_then(Route::repo);
    let why = repo.and_then(|r| refused(state, r, WHAT));
    out.push(row(
        "New",
        Action::NewHere,
        "New milestone",
        "new",
        why.clone(),
    ));
    let m = picked(state);
    let row_why = why.or_else(|| m.is_none().then(|| "no milestone is selected".to_owned()));
    let title = m.as_ref().map_or("the milestone", |m| m.title.as_str());
    let close = match &m {
        Some(m) if m.closed => format!("Reopen {title}"),
        _ => format!("Close {title}"),
    };
    out.push(row(
        "Edit",
        Action::EditComment,
        format!("Edit {title}"),
        "edit",
        row_why.clone(),
    ));
    out.push(row("Edit", Action::Close, close, "close", row_why.clone()));
    out.push(row(
        "Edit",
        Action::DeleteComment,
        format!("Delete {title}"),
        "",
        row_why,
    ));
}

/// The form's page.
pub(super) fn build(state: &State, page: &mut Page, route: &Route) {
    let Route::Housekeeping(of @ FormOf::Milestone { repo, number }) = route else {
        return;
    };
    let f = form(state, route);
    let heading = match number {
        Some(_) => format!("Edit the milestone in {repo}"),
        None => format!("New milestone in {repo}"),
    };
    let mut status = Vec::new();
    if f.get(Field::Due).is_empty() {
        status.push(("No due date: it's sorted last".to_owned(), Mark::Fine));
    }
    let write = state.first_key(Action::Comment);
    let save_key = state.first_key(Action::Create);
    let composing = matches!(
        &state.overlay,
        Some(crate::state::Overlay::Compose(c))
            if c.target == crate::review::ComposeTarget::Write(crate::writes::Text::Housekeeping(route.clone()))
    );
    let body = if composing { "" } else { f.text.as_str() };
    let view = WriteFormView {
        heading,
        base: Some(crate::ui::markdown::LinkBase::new(repo, "HEAD", "")),
        rows: vec![
            form_row(f, Field::Title, "Title"),
            form_row(f, Field::Due, "No due date"),
        ],
        status,
        boxes: vec![TextBox {
            title: "Description".into(),
            untitled: "Description",
            body,
            markdown: true,
            empty: "No description.",
            hint: "In Markdown (ctrl-e for $EDITOR)",
            link: Link::Comment,
            actions: Vec::new(),
        }],
        after: Vec::new(),
        write: &write,
        button: super::button(of, f),
        save_key: &save_key,
        why_not: super::problem(state, route, of),
        error: f.error.as_deref(),
    };
    crate::ui::pages::write_form(page, &view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_dates_are_days_of_the_calendar() {
        assert!(valid_date("2026-02-28"));
        assert!(valid_date("2028-02-29"));
        assert!(!valid_date("2026-02-29"));
        assert!(!valid_date("2026-13-01"));
        assert!(!valid_date("26-01-01"));
        assert!(!valid_date("tomorrow"));
    }
}
