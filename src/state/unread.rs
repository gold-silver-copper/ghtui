//! The header's unread count ([`DataKey::Unread`]): polled as often as
//! GitHub asks, and again after a change to your notifications.

use super::*;
use crate::api::notifications::Unread;
use crate::data::Picked;

/// How long to wait before polling again after a failed poll.
const RETRY_SECS: u64 = 60;

impl State {
    /// GitHub's count of the unread, unless the last poll failed: a count
    /// that may be wrong isn't shown.
    fn unread(&self) -> Option<&Unread> {
        let remote = self.data.get(&DataKey::Unread)?;
        (remote.error.is_none())
            .then(|| remote.data.as_ref().and_then(Unread::pick))
            .flatten()
    }

    /// The count came in: it's asked again when GitHub says to. While the
    /// TUI is suspended for `$EDITOR` the runtime handles no message, so
    /// the poll waits, and runs once it's back.
    #[must_use]
    pub(super) fn unread_polled(&mut self) -> Vec<Cmd> {
        let secs = self.unread().map_or(RETRY_SECS, |u| u.poll_secs);
        self.unread_poll += 1;
        let ms = secs.max(1).saturating_mul(1000);
        vec![Cmd::Timer(Timer::Unread(self.unread_poll), ms)]
    }

    /// Polls the unread count, unless a later poll (after a change)
    /// already set its own timer.
    #[must_use]
    pub(super) fn unread_due(&mut self, poll: u64) -> Vec<Cmd> {
        if poll == self.unread_poll {
            self.ensure(DataKey::Unread, true)
        } else {
            Vec::new()
        }
    }

    /// The header's unread count, if any are: `3`, or `50+`.
    pub fn unread_label(&self) -> Option<String> {
        let unread = self.unread().filter(|u| u.count > 0)?;
        let plus = if unread.more { "+" } else { "" };
        Some(format!("● {}{plus} unread", unread.count))
    }
}
