//! Whether GitHub can be reached. A request that can't connect (no
//! network, the server down, DNS failing) marks GitHub unreachable; any
//! answer at all marks it reachable again. While it's unreachable,
//! requests fail at once ([`ApiError::Offline`]) instead of waiting out
//! their timeouts, so nothing on screen loads forever, and the cache
//! answers instead. Only [`GitHub::probe`] asks meanwhile, to find out
//! when it's back.
//!
//! An [`ApiError::Offline`] always means the request never left: nothing
//! was sent, so a write that failed so can be queued and sent later
//! without being sent twice.

use std::sync::Arc;

use tokio::sync::watch;

use crate::api::{ApiError, GitHub};

/// Since when GitHub has been unreachable (Unix seconds), if it is.
/// Cheap to clone; clones share one state, and anyone may watch it.
#[derive(Debug, Clone)]
pub struct Reach(Arc<watch::Sender<Option<u64>>>);

impl Default for Reach {
    fn default() -> Self {
        Self(Arc::new(watch::Sender::new(None)))
    }
}

impl Reach {
    /// Since when GitHub has been unreachable, if it is.
    pub fn offline_since(&self) -> Option<u64> {
        *self.0.borrow()
    }

    /// A request couldn't connect.
    pub(crate) fn lost(&self) {
        self.0.send_if_modified(|since| {
            let was = since.is_some();
            since.get_or_insert_with(|| crate::store::now().max(1));
            !was
        });
    }

    /// GitHub answered.
    pub(crate) fn found(&self) {
        self.0.send_if_modified(|since| since.take().is_some());
    }

    /// Follows the changes from now on.
    pub fn watch(&self) -> watch::Receiver<Option<u64>> {
        self.0.subscribe()
    }
}

impl GitHub {
    /// Since when GitHub has been unreachable, if it is.
    pub fn offline_since(&self) -> Option<u64> {
        self.reach().offline_since()
    }

    /// Asks GitHub whether it's there: any answer at all (even a refusal)
    /// is. Unlike other requests, it's sent while GitHub is unreachable.
    pub async fn probe(&self) -> bool {
        match self.probe_once().await {
            Ok(()) => true,
            Err(err) => {
                tracing::debug!(%err, "GitHub still can't be reached");
                false
            }
        }
    }
}

/// What a request that never left says.
pub(crate) fn offline(err: &dyn std::fmt::Display) -> ApiError {
    ApiError::Offline(err.to_string())
}
