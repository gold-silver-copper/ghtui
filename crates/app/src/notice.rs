//! The status bar's notice, and the messages list of every one shown.

use std::collections::VecDeque;

use ghtui_ui::bars::Notice;

use crate::state::{Cmd, Timer};

/// How many notices the messages list keeps.
const KEPT: usize = 50;

/// What the status bar says. Saying is the only way to show a notice, and
/// each saying is numbered, so it gets a timer and a messages entry of its
/// own and only its own timer clears it.
#[derive(Default)]
pub struct Notices {
    /// The notice showing, and its number.
    shown: Option<(u64, Notice)>,
    /// The last number given, and the last given a timer.
    said: u64,
    timed: u64,
    /// Notices shown this session, newest first, with when (Unix seconds).
    log: VecDeque<(u64, Notice)>,
}

impl Notices {
    /// Says `text` for a few seconds.
    pub fn info(&mut self, text: impl Into<String>) {
        self.say(Notice::Info(text.into()));
    }

    /// Says what went wrong, for longer.
    pub fn error(&mut self, text: impl Into<String>) {
        self.say(Notice::Error(text.into()));
    }

    /// Shows `notice`, under a new number.
    pub fn say(&mut self, notice: Notice) {
        self.said += 1;
        self.shown = Some((self.said, notice));
    }

    /// Clears the notice showing, and gives it back.
    pub fn dismiss(&mut self) -> Option<Notice> {
        self.shown.take().map(|(_, notice)| notice)
    }

    /// Notice number `id`'s time is up: cleared, if it's still showing.
    pub fn expire(&mut self, id: u64) {
        self.shown = self.shown.take().filter(|(shown, _)| *shown != id);
    }

    pub fn shown(&self) -> Option<&Notice> {
        self.shown.as_ref().map(|(_, notice)| notice)
    }

    pub fn log(&self) -> &VecDeque<(u64, Notice)> {
        &self.log
    }

    /// The timer for a notice said since the last call (errors stay
    /// longer), which is logged as shown at `clock`'s now.
    pub fn timer(&mut self, clock: fn() -> u64) -> Option<Cmd> {
        let (id, notice) = self.shown.as_ref().filter(|(id, _)| *id != self.timed)?;
        self.timed = *id;
        self.log.push_front((clock(), notice.clone()));
        self.log.truncate(KEPT);
        let ms = match notice {
            Notice::Info(_) => 4_000,
            Notice::Error(_) => 10_000,
        };
        Some(Cmd::Timer(Timer::ExpireNotice(*id), ms))
    }
}
