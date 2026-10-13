//! Whose cache and drafts these are: one account on one GitHub. The cache
//! and the drafts keep each account's apart, under `<host>/<login>/`. Until
//! GitHub says whose the token is (on its first use), it's
//! `<host>/token-<fingerprint>/`, and the store remembers the login for
//! that fingerprint next time.

use std::sync::{Arc, PoisonError, RwLock};

/// Cheap to clone; all clones follow the same account.
#[derive(Debug, Clone, Default)]
pub struct Account(Arc<RwLock<Who>>);

#[derive(Debug, Default)]
struct Who {
    host: String,
    /// The token's fingerprint (never the token).
    fingerprint: String,
    login: Option<String>,
}

impl Account {
    pub fn new(host: &str, fingerprint: &str, login: Option<String>) -> Self {
        Self(Arc::new(RwLock::new(Who {
            host: host.to_owned(),
            fingerprint: fingerprint.to_owned(),
            login,
        })))
    }

    /// What the account's keys and paths start with: `<host>/<login>/`,
    /// or nothing for a store that isn't anyone's (in tests).
    pub fn scope(&self) -> String {
        let who = self.0.read().unwrap_or_else(PoisonError::into_inner);
        match &who.login {
            _ if who.host.is_empty() => String::new(),
            Some(login) => format!("{}/{login}/", who.host),
            None => format!("{}/token-{}/", who.host, who.fingerprint),
        }
    }

    /// Whose token it is, as last seen: known before GitHub answers, from
    /// the last time it did.
    pub fn login(&self) -> Option<String> {
        let who = self.0.read().unwrap_or_else(PoisonError::into_inner);
        who.login.clone()
    }

    /// Where the login for this token is remembered.
    pub(crate) fn token_key(&self) -> Option<String> {
        let who = self.0.read().unwrap_or_else(PoisonError::into_inner);
        (!who.host.is_empty()).then(|| format!("{}/{}", who.host, who.fingerprint))
    }

    /// The token is `login`'s: from now on, its scope. Whether that's new.
    pub(crate) fn sign_in(&self, login: &str) -> bool {
        let mut who = self.0.write().unwrap_or_else(PoisonError::into_inner);
        if who.host.is_empty() || who.login.as_deref() == Some(login) {
            return false;
        }
        who.login = Some(login.to_owned());
        true
    }
}
