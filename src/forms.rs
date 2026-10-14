//! Forms on pages, and what was sent from them.
//!
//! A form is written on its page: a new issue or pull request
//! ([`crate::create`]), a release ([`crate::release`]), a gist
//! ([`crate::gist`]), a wiki page ([`crate::wiki::form`]),
//! housekeeping's ([`crate::housekeeping`]), and running a workflow by
//! hand ([`crate::workflows`]). They're kept in one map by their page, so
//! leaving one keeps it, each with what it [`Sent`]. A change to one is
//! a new version of it, which is what its page is built again for.
//!
//! A form's write may ask first ([`Ask`], answered with an [`Answer`]).
//! The composer, a question and a confirmation say what they sent the
//! same way; [`say`], [`sending`] and [`close_sender`] reach whichever
//! one is open.

use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui_textarea::CursorMove;

use crate::compose::Target;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::route::Route;
use crate::state::{Cmd, Overlay, State};
use crate::ui::form::Row;
use crate::ui::page::Link;

/// What a kind of form does, on its pages ([`crate::route::Family::form`]).
pub struct Kind {
    /// `c`, or Enter on part `part` of its text: the composer on it.
    pub compose: fn(&mut State, usize) -> Vec<Cmd>,
    /// Enter on one of its rows.
    pub row: fn(&mut State, &Row) -> Vec<Cmd>,
    /// `,s`, ctrl-s or its button: makes it, saves it, or asks first.
    pub submit: fn(&mut State) -> Vec<Cmd>,
    /// Brings it up to date with what's arrived: filled in from GitHub's
    /// data once, and the like.
    pub follow: fn(&mut State) -> Vec<Cmd>,
    /// The menu's rows on its page.
    pub offers: fn(&State, &mut Vec<Doable>),
}

/// The form on screen, if a form's page is.
fn here(state: &State) -> Option<&'static Kind> {
    state.route()?.family().form()
}

/// What forms add: making or saving the one on screen, and writing its
/// text.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &[Action::Create],
    takes: |state, action| action == Action::Comment && here(state).is_some(),
    act: |state, action| match action {
        Action::Comment => compose_here(state),
        _ => submit(state),
    },
    offers,
    follow,
};

/// `,s` or ctrl-s on a form's page.
#[must_use]
pub fn submit(state: &mut State) -> Vec<Cmd> {
    let Some(kind) = here(state) else {
        return state.just_say("That makes or saves a form, on its page");
    };
    if state.route().is_some_and(|r| state.forms.sent(r).sending) {
        say(state, "It's being sent".into());
        return Vec::new();
    }
    (kind.submit)(state)
}

/// `c` on a form's page: the composer on the text selected (the first,
/// when it's a row of another).
#[must_use]
pub fn compose_here(state: &mut State) -> Vec<Cmd> {
    let Some(kind) = here(state) else {
        return Vec::new();
    };
    let selected = match state.screen() {
        crate::state::Screen::Page(p) => p.selected_link(),
        crate::state::Screen::Diff(_) => None,
    };
    let part = match selected {
        Some(Link::Form(Row::Text(i) | Row::Rename(i) | Row::Remove(i))) => *i,
        _ => 0,
    };
    (kind.compose)(state, part)
}

/// Enter on one of a form's rows.
#[must_use]
pub fn row(state: &mut State, row: &Row) -> Vec<Cmd> {
    match (row, here(state)) {
        (Row::New, _) => crate::state::apply(state, Action::NewHere),
        (Row::Text(part), Some(kind)) => (kind.compose)(state, *part),
        (Row::Save, Some(_)) => submit(state),
        (_, Some(kind)) => (kind.row)(state, row),
        (_, None) => Vec::new(),
    }
}

/// After GitHub's data arrives, or a form's page opens: brings the form
/// on screen up to date.
#[must_use]
pub fn follow(state: &mut State) -> Vec<Cmd> {
    here(state).map_or_else(Vec::new, |kind| (kind.follow)(state))
}

/// The menu's rows for the form on screen.
fn offers(state: &State, out: &mut Vec<Doable>) {
    if let Some(kind) = here(state) {
        (kind.offers)(state, out);
    }
}

/// The menu's rows for the form on screen: writing its text (labelled
/// `write`, for a form with text), and saving it (`save`: its label and
/// short name), unless it's being sent or `why` it can't be.
pub(crate) fn offer(
    state: &State,
    write: Option<&str>,
    save: (&str, &'static str),
    why: Option<String>,
    out: &mut Vec<Doable>,
) {
    let sending = state.route().is_some_and(|r| state.forms.sent(r).sending);
    let busy = sending.then(|| "it's being sent".to_owned());
    if let Some(label) = write {
        let row = crate::nav::row("This page", Action::Comment, label, "write", busy.clone());
        out.push(row);
    }
    let row = crate::nav::row("This page", Action::Create, save.0, save.1, busy.or(why));
    out.push(row);
}

/// Whether GitHub (or git) is still answering what was sent, and why it
/// refused when it did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sent {
    pub sending: bool,
    pub error: Option<String>,
}

impl Sent {
    const NONE: Sent = Sent {
        sending: false,
        error: None,
    };

    /// Sent: waiting for the answer.
    pub fn start(&mut self) {
        *self = Sent {
            sending: true,
            error: None,
        };
    }

    /// Refused, and why.
    pub fn fail(&mut self, why: String) {
        *self = Sent {
            sending: false,
            error: Some(why),
        };
    }
}

/// A form's fields, as written so far.
pub trait Fields: Any + Send {
    /// Part `part` of its text, as the composer writes it (most forms have
    /// one part, some none).
    fn text(&mut self, _part: usize) -> Option<&mut String> {
        None
    }

    /// The composer's title on part `part`, on the form's page `route`.
    fn title(&self, _route: &Route, _part: usize) -> String {
        String::new()
    }

    /// What ctrl-s in the composer does, for its hint.
    fn save(&self, _route: &Route) -> &'static str {
        "save"
    }
}

/// What only says it was sent.
impl Fields for () {}

struct Entry {
    version: u64,
    sent: Sent,
    fields: Box<dyn Fields>,
}

/// The forms being written, by their page.
#[derive(Default)]
pub struct Forms {
    map: HashMap<Route, Entry>,
    /// The last version given out: never given again, so a form taken
    /// and begun anew is a new version.
    stamp: u64,
}

impl Forms {
    /// The form at `route`, if one's begun there.
    pub fn get<F: Fields>(&self, route: &Route) -> Option<&F> {
        let fields: &dyn Any = self.map.get(route)?.fields.as_ref();
        fields.downcast_ref()
    }

    /// The form at `route` as its page shows it: blank until it's begun.
    pub fn shown<F: Fields + Clone + Default>(&self, route: &Route) -> Cow<'_, F> {
        self.get(route)
            .map_or_else(|| Cow::Owned(F::default()), Cow::Borrowed)
    }

    fn entry(&mut self, route: &Route) -> &mut Entry {
        self.stamp += 1;
        let entry = (self.map.entry(route.clone())).or_insert_with(|| Entry {
            version: 0,
            sent: Sent::default(),
            fields: Box::new(()),
        });
        entry.version = self.stamp;
        entry
    }

    /// The form at `route`, to change: begun blank if it wasn't.
    #[expect(
        clippy::expect_used,
        reason = "the entry holds an F: it was made one just before"
    )]
    pub fn edit<F: Fields + Default>(&mut self, route: &Route) -> &mut F {
        let entry = self.entry(route);
        if !(entry.fields.as_ref() as &dyn Any).is::<F>() {
            entry.fields = Box::new(F::default());
        }
        let fields: &mut dyn Any = entry.fields.as_mut();
        fields.downcast_mut().expect("an F")
    }

    /// The form at `route`, to change, if one's begun there.
    pub fn get_mut<F: Fields>(&mut self, route: &Route) -> Option<&mut F> {
        self.get::<F>(route)?;
        let fields: &mut dyn Any = self.entry(route).fields.as_mut();
        fields.downcast_mut()
    }

    fn fields(&self, route: &Route) -> Option<&dyn Fields> {
        Some(self.map.get(route)?.fields.as_ref())
    }

    fn fields_mut(&mut self, route: &Route) -> Option<&mut dyn Fields> {
        self.map.get(route)?;
        Some(self.entry(route).fields.as_mut())
    }

    /// The form at `route`, taken away: blank if there was none.
    pub fn take<F: Fields + Default>(&mut self, route: &Route) -> F {
        let fields = self.map.remove(route).map(|e| e.fields as Box<dyn Any>);
        (fields.and_then(|f| f.downcast().ok())).map_or_else(F::default, |f| *f)
    }

    /// What was sent from the form at `route`.
    pub fn sent(&self, route: &Route) -> &Sent {
        self.map.get(route).map_or(&Sent::NONE, |e| &e.sent)
    }

    /// What was sent from the form at `route`, to change.
    pub fn sent_mut(&mut self, route: &Route) -> &mut Sent {
        &mut self.entry(route).sent
    }

    /// The form at `route`'s version: it changes whenever the form does.
    pub fn version(&self, route: &Route) -> u64 {
        self.map.get(route).map_or(0, |e| e.version)
    }
}

/// The composer on a form: part `part` of the form at `route`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Text {
    pub route: Route,
    pub part: usize,
}

impl Text {
    pub fn title(&self, state: &State) -> String {
        let fields = state.forms.fields(&self.route);
        fields
            .map(|f| f.title(&self.route, self.part))
            .unwrap_or_default()
    }

    pub fn save(&self, state: &State) -> &'static str {
        let fields = state.forms.fields(&self.route);
        fields.map_or("save", |f| f.save(&self.route))
    }

    /// What's written, kept with the form (esc, or before saving).
    pub fn keep(&self, state: &mut State, written: String) {
        let forms = &mut state.forms;
        if let Some(text) = forms
            .fields_mut(&self.route)
            .and_then(|f| f.text(self.part))
        {
            *text = written;
        }
        forms.sent_mut(&self.route).error = None;
    }

    /// ctrl-s: keeps what's written, and saves the form on screen.
    #[must_use]
    pub fn save_form(&self, state: &mut State, written: String) -> Vec<Cmd> {
        self.keep(state, written);
        if state.route() != Some(&self.route) {
            return Vec::new();
        }
        crate::state::apply(state, Action::Create)
    }
}

/// Where writing starts in a text whose title is its first line: the
/// title's end.
pub(crate) const TITLE: &[CursorMove] = &[CursorMove::Top, CursorMove::End];

/// Opens the composer on part `part` of the form at `route`, with what's
/// written there, moved by `cursor` to where writing starts.
#[must_use]
pub(crate) fn compose<F: Fields + Default>(
    state: &mut State,
    route: &Route,
    part: usize,
    cursor: &[CursorMove],
) -> Vec<Cmd> {
    if state.forms.sent(route).sending {
        return state.just_say("It's being sent");
    }
    // Its box on the page makes way.
    let Some(text) = state.forms.edit::<F>(route).text(part).cloned() else {
        return Vec::new();
    };
    let target = Target::Form(Text {
        route: route.clone(),
        part,
    });
    let cmds = state.compose(target, &text);
    if let Some(Overlay::Compose(c)) = &mut state.overlay {
        for &to in cursor {
            c.input.move_cursor(to);
        }
    }
    cmds
}

/// Whether the composer is open on part `part` of the form at `route`:
/// its box on the page makes way.
pub(crate) fn composing(state: &State, route: &Route, part: usize) -> bool {
    matches!(&state.overlay, Some(Overlay::Compose(c))
        if matches!(&c.target, Target::Form(t) if t.route == *route && t.part == part))
}

impl Overlay {
    /// What this overlay sent, for one that sends.
    pub fn sent_mut(&mut self) -> Option<&mut Sent> {
        match self {
            Overlay::Compose(c) => Some(&mut c.sent),
            Overlay::Ask(a) => Some(&mut a.sent),
            Overlay::Confirm(c) => Some(&mut c.sent),
            Overlay::Submit(d) => Some(&mut d.sent),
            Overlay::Find(_)
            | Overlay::Search(_)
            | Overlay::Hints(_)
            | Overlay::Menu(_)
            | Overlay::Picker(_) => None,
        }
    }
}

/// Says `text` where it'll be seen: in what sent it when that's open
/// (the composer, a question, a confirmation), else in the status bar.
pub(crate) fn say(state: &mut State, text: String) {
    match state.overlay.as_mut().and_then(Overlay::sent_mut) {
        Some(sent) => sent.fail(text),
        None => state.error(text),
    }
}

/// Marks what sent something (the composer, a question, a
/// confirmation) as waiting for GitHub.
pub(crate) fn sending(state: &mut State) {
    if let Some(sent) = state.overlay.as_mut().and_then(Overlay::sent_mut) {
        sent.start();
    }
}

/// Marks the form at `route`, and what sent it, as waiting for GitHub.
pub(crate) fn form_sending(state: &mut State, route: &Route) {
    state.forms.sent_mut(route).start();
    sending(state);
}

/// GitHub refused what the form at `route` sent: the form says why, as
/// does what sent it (else the status bar).
pub(crate) fn failed(state: &mut State, route: &Route, why: String) {
    if state.forms.map.contains_key(route) {
        state.forms.sent_mut(route).fail(why.clone());
    }
    say(state, why);
}

/// Closes the composer or the question that sent something from
/// `route`, once it's done.
pub(crate) fn close_sender(state: &mut State, route: &Route) {
    let ours = match &state.overlay {
        Some(Overlay::Compose(c)) => matches!(&c.target, Target::Form(t) if t.route == *route),
        Some(Overlay::Ask(a)) => a.route == *route,
        _ => false,
    };
    if ours {
        state.overlay = None;
    }
}

/// The form at `route` was made: it's taken away, and what sent it
/// closes.
pub(crate) fn done<F: Fields + Default>(state: &mut State, route: &Route) -> F {
    close_sender(state, route);
    state.forms.take(route)
}

// ---- questions -------------------------------------------------------------------

/// A question before a form's write is made: what, what's worth knowing,
/// and the ways to go ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    pub route: Route,
    pub title: String,
    pub facts: Vec<(String, bool)>,
    pub choices: Vec<(String, Answer)>,
    pub selected: usize,
    pub sent: Sent,
}

impl Ask {
    pub fn new(
        route: Route,
        title: String,
        facts: Vec<(String, bool)>,
        choices: Vec<(String, Answer)>,
    ) -> Self {
        Self {
            route,
            title,
            facts,
            choices,
            selected: 0,
            sent: Sent::default(),
        }
    }

    /// What's happening while git or GitHub answers.
    pub fn busy(&self) -> Option<&'static str> {
        let (_, answer) = self
            .choices
            .get(self.selected)
            .filter(|_| self.sent.sending)?;
        Some(answer.busy())
    }
}

/// The ways to answer a question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// A new pull request's branch, pushed ([`crate::create`]).
    Push,
    CreatePr {
        draft: bool,
    },
    /// A release ([`crate::release`]).
    Release {
        draft: bool,
    },
    DeleteRelease,
    /// The deleted release's tag too.
    DeleteTag,
    KeepTag,
    /// The release's asset of this name.
    DeleteAsset(String),
    /// A gist ([`crate::gist`]).
    Gist {
        public: bool,
    },
    DeleteGist,
    Site(crate::site::Answer),
    Housekeeping(crate::housekeeping::Answer),
}

impl Answer {
    /// What's happening while git or GitHub answers.
    pub fn busy(&self) -> &'static str {
        match self {
            Answer::Push => "Pushing…",
            Answer::CreatePr { .. } => "Creating the pull request…",
            Answer::Release { draft: true } => "Saving the draft…",
            Answer::Release { .. } => "Saving the release…",
            Answer::DeleteRelease => "Deleting the release…",
            Answer::DeleteTag => "Deleting the tag…",
            Answer::KeepTag => "",
            Answer::DeleteAsset(_) => "Deleting the asset…",
            Answer::Gist { .. } => "Creating the gist…",
            Answer::DeleteGist => "Deleting the gist…",
            Answer::Site(a) => a.busy(),
            Answer::Housekeeping(a) => a.busy(),
        }
    }
}

/// Asks `title` about `route`, with what's worth knowing and the ways to
/// go ahead.
pub(crate) fn ask(
    state: &mut State,
    route: Route,
    title: String,
    facts: Vec<(String, bool)>,
    choices: Vec<(String, Answer)>,
) {
    let ask = Ask::new(route, title, facts, choices);
    state.overlay = Some(Overlay::Ask(Box::new(ask)));
}

/// Keys in the question: choose, go ahead, or cancel.
#[must_use]
pub fn on_ask_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Ask(ask)) = &mut state.overlay else {
        return Vec::new();
    };
    let n = ask.choices.len().max(1);
    match key.code {
        KeyCode::Esc => state.overlay = None,
        _ if ask.sent.sending => {}
        KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => ask.selected = (ask.selected + 1) % n,
        KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
            ask.selected = (ask.selected + n - 1) % n;
        }
        KeyCode::Enter => {
            let Some((_, answer)) = ask.choices.get(ask.selected).cloned() else {
                return Vec::new();
            };
            let route = ask.route.clone();
            return answered(state, &route, answer);
        }
        _ => {}
    }
    Vec::new()
}

/// The question about `route` was answered.
#[must_use]
fn answered(state: &mut State, route: &Route, answer: Answer) -> Vec<Cmd> {
    match answer {
        Answer::Push => crate::create::answer(state, route, None),
        Answer::CreatePr { draft } => crate::create::answer(state, route, Some(draft)),
        Answer::Release { draft } => crate::release::create(state, route, draft),
        Answer::DeleteRelease => crate::release::delete(state, route),
        Answer::DeleteTag => crate::release::delete_tag(state, route),
        Answer::KeepTag => {
            close_sender(state, route);
            if let Route::Releases(crate::route::releases::Release { tag, .. }) = route {
                state.info(format!("Kept the tag {tag}"));
            }
            Vec::new()
        }
        Answer::DeleteAsset(name) => crate::release::assets::delete(state, &name),
        Answer::Gist { public } => crate::gist::create(state, route, public),
        Answer::DeleteGist => crate::gist::delete(state, route),
        Answer::Site(a) => crate::site::answer(state, route, a),
        Answer::Housekeeping(a) => crate::housekeeping::answer(state, route, a),
    }
}
