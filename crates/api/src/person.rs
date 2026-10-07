//! Who wrote or committed a commit. A module of its own so that [`Login`]'s
//! field is private to it: a decoder can't make one from any string at hand.

use serde::{Deserialize, Serialize};

use crate::browse::GitActor;

/// Who wrote or committed a commit, as GitHub knows them. Kept externally
/// tagged in the cache: a login and a git name must not read back alike.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Person {
    /// The account GitHub linked to the commit's email: the only kind of
    /// person a profile link may be built from.
    User(Login),
    /// Only git's name for them, with no account behind it.
    Git(String),
    /// No author, or an empty name.
    Unknown,
}

impl Person {
    pub fn login(&self) -> Option<&str> {
        match self {
            Person::User(Login(login)) => Some(login),
            _ => None,
        }
    }

    /// The text to show. Not a login: never pass it to a URL builder.
    pub fn name(&self) -> &str {
        match self {
            Person::User(Login(name)) | Person::Git(name) => name,
            Person::Unknown => "unknown",
        }
    }
}

/// The login GitHub linked to a commit's email. Made only from a
/// [`GitActor`]'s `user`, by [`Login::unchecked`], or read back from the
/// cache (`Deserialize`, which only the store should use).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Login(String);

impl Login {
    /// A login that didn't come from GitHub: for fixtures and tests.
    pub fn unchecked(login: impl Into<String>) -> Login {
        Login(login.into())
    }
}

impl From<Option<GitActor>> for Person {
    fn from(actor: Option<GitActor>) -> Person {
        match actor {
            Some(GitActor { user: Some(u), .. }) => Person::User(Login(u.login)),
            Some(GitActor { name: Some(n), .. }) if !n.is_empty() => Person::Git(n),
            _ => Person::Unknown,
        }
    }
}
