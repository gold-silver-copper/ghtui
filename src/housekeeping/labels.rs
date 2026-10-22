//! A repository's labels: made, edited (name, colour, description) and
//! deleted, as on GitHub's labels page.

use crate::api::browse::Results;
use crate::api::housekeeping::{LabelChange, LabelInfo};
use crate::api::model::RepoId;
use crate::route::{Route, issues};
use crate::ui::page::Page;
use crate::ui::pages::{Mark, WriteFormView};

use super::{
    Expected, Field, Form, FormOf, Subject, ask, expect, form, form_mut, form_row, item, refused,
    selected, setting,
};
use crate::data::DataKey;
use crate::forms::say;
use crate::state::{Cmds, State};
use crate::work::{self, Cx, Failure, Job};

/// Makes or changes a label (`was`: its name before), or (no change)
/// deletes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Save {
    pub repo: RepoId,
    pub was: Option<String>,
    pub change: Option<LabelChange>,
}

/// The colours GitHub offers for a new label.
const COLOURS: &[(&str, &str)] = &[
    ("b60205", "red"),
    ("d93f0b", "orange"),
    ("fbca04", "yellow"),
    ("0e8a16", "green"),
    ("006b75", "teal"),
    ("1d76db", "blue"),
    ("0052cc", "dark blue"),
    ("5319e7", "purple"),
    ("e99695", "pink"),
    ("f9d0c4", "peach"),
    ("fef2c0", "cream"),
    ("c2e0c6", "mint"),
    ("bfdadc", "pale teal"),
    ("c5def5", "pale blue"),
    ("bfd4f2", "lavender"),
    ("d4c5f9", "lilac"),
];

/// The repository's labels, as loaded.
pub(super) fn list<'a>(state: &'a State, repo: &RepoId) -> Option<&'a Results<LabelInfo>> {
    state.picked(&DataKey::Labels(repo.clone()))
}

/// The label selected on the labels page.
pub(super) fn picked(state: &State) -> Option<&LabelInfo> {
    let Some(Route::Issues(issues::Labels(repo))) = state.route() else { return None };
    let list = list(state, repo)?;
    selected(state, &list.items, |l| {
        crate::ui::pages::url::label(repo, &l.name)
    })
}

/// `,o` or `,e`: the label's form.
pub(super) fn open(state: &mut State, repo: &RepoId, name: Option<String>) -> Cmds {
    if let Some(why) = refused(state, repo, "Changing labels") {
        return state.just_say(why);
    }
    super::open(
        state,
        FormOf::Label {
            repo: repo.clone(),
            name,
        },
    )
}

/// `,e` on the labels page: the selected label's form.
pub(super) fn edit(state: &mut State, repo: &RepoId) -> Cmds {
    let Some(name) = picked(state).map(|l| l.name.clone()) else {
        return state.just_say("Select a label first");
    };
    open(state, repo, Some(name))
}

/// Fills the form from the label once the list has it; a new one gets a
/// colour of GitHub's.
pub(super) fn follow(
    state: &mut State,
    route: &Route,
    repo: &RepoId,
    name: Option<&String>,
) -> Cmds {
    if form(state, route).started {
        return Cmds::new();
    }
    let find = |name: &String| {
        let found = list(state, repo)?.items.iter().find(|l| l.name == *name);
        found.cloned()
    };
    // An edited label's form waits for it.
    let label = match name.map(find) {
        Some(None) => return Cmds::new(),
        label => label.flatten(),
    };
    // A colour of GitHub's, picked by the time: new labels differ.
    let n = usize::try_from((state.clock)() / 1000).unwrap_or_default();
    let colour = COLOURS.get(n % COLOURS.len()).map_or("ededed", |c| c.0);
    let f = form_mut(state, route);
    f.started = true;
    match label {
        Some(l) => {
            f.set(Field::Name, l.name);
            f.set(Field::Colour, l.color.to_lowercase());
            f.set(Field::Description, l.description);
        }
        None => f.set(Field::Colour, colour),
    }
    // A new label starts with its name.
    if name.is_none() && state.overlay.is_none() {
        return super::open_field(state, route, Field::Name);
    }
    Cmds::new()
}

/// `#rrggbb` (or without `#`, or `#rgb`) as GitHub keeps it.
fn colour(text: &str) -> Option<String> {
    let hex = text.trim().trim_start_matches('#');
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        6 => Some(hex.to_lowercase()),
        3 => Some(
            hex.chars()
                .flat_map(|c| [c, c])
                .collect::<String>()
                .to_lowercase(),
        ),
        _ => None,
    }
}

/// The colour picker's rows: a colour typed, then GitHub's.
pub(super) fn colour_rows(route: &Route, q: &str) -> crate::picker::Rows {
    let set = |c: &str| setting(route, Field::Colour, c);
    let mut rows = Vec::new();
    if !q.is_empty() {
        rows.push(match colour(q) {
            Some(c) => item(format!("#{c}"), "↵", set(&c)),
            None => item("A colour is #rrggbb: six hexadecimal digits", "", None),
        });
    }
    let q = q.to_lowercase();
    for (hex, name) in COLOURS {
        if q.is_empty() || name.contains(q.as_str()) || hex.starts_with(q.trim_start_matches('#')) {
            rows.push(item(format!("#{hex}"), *name, set(hex)));
        }
    }
    rows
}

/// The label as the form has it, or why it can't be made.
pub(super) fn change(f: &Form) -> Result<LabelChange, String> {
    let name = f.get(Field::Name).trim();
    if name.is_empty() {
        return Err("A label needs a name".into());
    }
    let Some(color) = colour(f.get(Field::Colour)) else {
        return Err("A label's colour is #rrggbb".into());
    };
    Ok(LabelChange {
        name: name.to_owned(),
        color,
        description: f.get(Field::Description).trim().to_owned(),
    })
}

/// Makes or saves the label: no question, as on GitHub.
pub(super) fn submit(state: &mut State, route: &Route, repo: &RepoId, was: Option<&str>) -> Cmds {
    if let Some(why) = refused(state, repo, "Changing labels") {
        return say(state, why);
    }
    let change = or_return!(change(&form(state, route)), |why| say(state, why));
    let taken = list(state, repo).is_some_and(|l| {
        (l.items.iter())
            .any(|l| l.name.eq_ignore_ascii_case(&change.name) && Some(l.name.as_str()) != was)
    });
    if taken {
        return say(
            state,
            format!("{repo} has a label named {} already", change.name),
        );
    }
    crate::forms::form_sending(state, route);
    state.info(format!("Saving the label {}…", change.name));
    work::start(Save {
        repo: repo.clone(),
        was: was.map(str::to_owned),
        change: Some(change),
    })
}

/// `,D` on the labels page: asks first, as GitHub does.
pub(super) fn ask_delete(state: &mut State, repo: &RepoId) -> Cmds {
    if let Some(why) = refused(state, repo, "Changing labels") {
        return state.just_say(why);
    }
    let Some((name, open)) = picked(state).map(|l| (l.name.clone(), l.open)) else {
        return state.just_say("Select a label first");
    };
    let on = match open {
        0 => "No open issue or pull request has it".to_owned(),
        1 => "It comes off its open issue or pull request (and any closed ones)".to_owned(),
        n => format!("It comes off its {n} open issues and pull requests (and any closed ones)"),
    };
    let title = format!("Delete the label {name}?");
    let subject = Subject::Label(repo.clone(), name);
    ask(
        state,
        title,
        vec![(on, open > 0), ("It can't be brought back".into(), true)],
        vec![("Delete label".into(), super::Answer::Delete(subject))],
    )
}

pub(super) fn delete(state: &mut State, repo: RepoId, name: String) -> Cmds {
    crate::forms::sending(state);
    work::start(Save {
        repo,
        was: Some(name),
        change: None,
    })
}

impl Job for Save {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        let (gh, repo) = (&cx.gh, &self.repo);
        match (&self.was, &self.change) {
            (None, Some(c)) => gh.create_label(repo, c).await?,
            (Some(was), Some(c)) => gh.update_label(repo, was, c).await?,
            (Some(was), None) => gh.delete_label(repo, was).await?,
            (None, None) => {}
        }
        Ok(())
    }

    /// GitHub's answer: the labels page follows it until it shows the change.
    fn done(self, state: &mut State, result: Result<(), Failure>) -> Cmds {
        let Save { repo, was, change } = self;
        let form = Route::Housekeeping(FormOf::Label {
            repo: repo.clone(),
            name: was.clone().filter(|_| change.is_some()),
        });
        let page = Route::Issues(issues::Labels(repo.clone()));
        if let Err(why) = result {
            return crate::forms::failed(state, &form, why.to_string());
        }
        crate::forms::close_sender(state, &page);
        crate::forms::done::<Form>(state, &form);
        let said = match (&change, &was) {
            (Some(c), None) => format!("Created the label {}", c.name),
            (Some(c), Some(_)) => format!("Saved the label {}", c.name),
            (None, Some(was)) => format!("Deleted the label {was}"),
            (None, None) => return Cmds::new(),
        };
        state.info(said);
        let mut cmds = Cmds::new();
        // The form goes back to the labels page.
        if state.route() == Some(&form) {
            cmds.extend(state.back());
            state.forward.clear();
            if state.route() != Some(&page) {
                cmds.extend(state.replace(page.clone(), true, None));
            }
        }
        if state.route() == Some(&page) {
            // Deleted or renamed: its old name is gone.
            let gone = was.filter(|was| change.as_ref().is_none_or(|c| c.name != *was));
            expect(
                state,
                page.clone(),
                Expected::Label {
                    repo,
                    gone,
                    shown: change,
                },
            );
            cmds.extend(state.ensure_route(&page, true));
        }
        cmds
    }
}

/// The form's page.
pub(super) fn build(state: &State, page: &mut Page, route: &Route) {
    let Route::Housekeeping(of @ FormOf::Label { repo, name }) = route else { return };
    let f = form(state, route);
    let heading = match name {
        Some(name) => format!("Edit the label {name} in {repo}"),
        None => format!("New label in {repo}"),
    };
    let mut status = Vec::new();
    if let Ok(c) = change(&f) {
        status.push((
            format!("Shows as  {}  (#{})", c.name, c.color),
            Mark::Fine,
            None,
        ));
    }
    let view = WriteFormView {
        heading,
        rows: vec![
            form_row(&f, Field::Name, "Label name"),
            form_row(&f, Field::Description, "Description (optional)"),
            form_row(&f, Field::Colour, "#rrggbb"),
        ],
        status,
        button: super::button(of, &f),
        why_not: super::problem(state, route, of),
        ..WriteFormView::default()
    };
    crate::forms::write_form(state, route, page, view);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_are_githubs_hex() {
        assert_eq!(colour("#D93F0B").as_deref(), Some("d93f0b"));
        assert_eq!(colour("abc").as_deref(), Some("aabbcc"));
        assert_eq!(colour("#12345"), None);
        assert_eq!(colour("red"), None);
    }
}
