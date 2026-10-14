//! Forms on pages, and what was sent from them.
//!
//! A form is written on its page: a new issue or pull request
//! ([`crate::create`]), a release ([`crate::release`]), a gist
//! ([`crate::gist`]), a wiki page ([`crate::wiki::form`]),
//! housekeeping's ([`crate::housekeeping`]), and running a workflow by
//! hand ([`crate::actions`]). They're kept in one map by their page, so
//! leaving one keeps it, each with what it [`Sent`]. A change to one is
//! a new version of it, which is what its page is built again for.
//!
//! The composer, a question and a confirmation say what they sent the
//! same way; [`say`], [`sending`] and [`close_sender`] reach whichever
//! one is open.

use std::any::Any;
use std::borrow::Cow;
use std::collections::HashMap;

use crate::route::Route;
use crate::state::{Overlay, State};

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
pub trait Fields: Any + Send {}

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

    /// The form at `route` shows something new, though its fields are
    /// the same (the composer opened on it).
    pub fn touch(&mut self, route: &Route) {
        self.entry(route);
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
        Some(Overlay::Compose(c)) => c.target.form() == Some(route),
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
