//! The header's unread count: polled as often as GitHub asks, and again
//! after a change to your notifications.

use super::*;

/// How long to wait before polling again after a failed poll.
const RETRY_SECS: u64 = 60;

impl State {
    /// GitHub's count of the unread: shown, and asked again when GitHub
    /// says to. While the TUI is suspended for `$EDITOR` the runtime
    /// handles no message, so the poll waits, and runs once it's back.
    #[must_use]
    pub(super) fn unread_polled(
        &mut self,
        result: Result<crate::api::notifications::Unread, ApiError>,
    ) -> Vec<Cmd> {
        let secs = match result {
            Ok(unread) => {
                self.unread = Some(unread);
                unread.poll_secs
            }
            // A count that may be wrong isn't shown.
            Err(err) => {
                tracing::warn!(%err, "could not count unread notifications");
                self.unread = None;
                RETRY_SECS
            }
        };
        self.unread_poll += 1;
        let ms = secs.max(1).saturating_mul(1000);
        vec![Cmd::Timer(Timer::Unread(self.unread_poll), ms)]
    }

    /// Polls the unread count, unless a later poll (after a change)
    /// already set its own timer.
    #[must_use]
    pub(super) fn unread_due(&self, poll: u64) -> Vec<Cmd> {
        if poll == self.unread_poll {
            vec![Cmd::Api(Api::FetchUnread)]
        } else {
            Vec::new()
        }
    }

    /// The header's unread count, if any are: `3`, or `50+`.
    pub fn unread_label(&self) -> Option<String> {
        let unread = self.unread.filter(|u| u.count > 0)?;
        let plus = if unread.more { "+" } else { "" };
        Some(format!("● {}{plus} unread", unread.count))
    }
}
