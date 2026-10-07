//! Remote data as a page sees it: loading, failed (and why), or ready.

use crate::page::{Page, Role, Seg};
use crate::pages::{flash, loading_box};

/// One need of a page, as exactly one of loading, failed (and why) or
/// ready. Pages can't look inside: [`Fetched::show`] says what isn't
/// ready, so a failure can't pass for "Loading…" or for "there's none".
/// Outside this crate only `Remote::fetched` makes one.
#[derive(Debug)]
pub struct Fetched<'a, T: ?Sized> {
    data: Option<&'a T>,
    /// Why it failed, without data and while not retrying.
    error: Option<&'a str>,
    /// The key that tries again.
    retry: &'a str,
}

impl<T: ?Sized> Clone for Fetched<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: ?Sized> Copy for Fetched<'_, T> {}

impl<'a, T: ?Sized> Fetched<'a, T> {
    /// Data a page already has (a part of another need).
    pub(crate) fn ready(data: &'a T) -> Self {
        let (data, error, retry) = (Some(data), None, "");
        Self { data, error, retry }
    }

    /// From how a fetch is going: its data (even a stale copy) is ready,
    /// else its error once it isn't retrying, else it's loading. Only the
    /// owner of a fetch's state calls this.
    pub fn new_unchecked(
        data: Option<&'a T>,
        loading: bool,
        error: Option<&'a str>,
        retry: &'a str,
    ) -> Self {
        let error = error.filter(|_| data.is_none() && !loading);
        Self { data, error, retry }
    }

    /// The part of the data `f` picks, failing when it isn't there. A
    /// plain `fn`, so it can't pick data from outside the fetch.
    pub fn pick<U: ?Sized>(self, f: fn(&'a T) -> Option<&'a U>) -> Fetched<'a, U> {
        let picked = self.data.map(f);
        let wrong = matches!(picked, Some(None)).then_some("the wrong kind of data");
        let (data, error, retry) = (picked.flatten(), self.error.or(wrong), self.retry);
        Fetched { data, error, retry }
    }

    /// The data, or `None` once the page says it's loading or why it
    /// failed: a skeleton box on an empty page, else "Loading `what`…".
    pub fn show(self, page: &mut Page, what: &str) -> Option<&'a T> {
        self.show_or(page, what, |page| {
            if page.lines.is_empty() {
                loading_box(page, vec![Seg::new("Loading…", Role::Meta)]);
            } else {
                page.line(vec![Seg::new(format!("Loading {what}…"), Role::Meta)]);
            }
        })
    }

    /// [`Fetched::show`], drawing `loading` while it loads.
    pub fn show_or(
        self,
        page: &mut Page,
        what: &str,
        loading: impl FnOnce(&mut Page),
    ) -> Option<&'a T> {
        #[expect(clippy::disallowed_methods, reason = "said here")]
        match (self.error.is_some(), self.text(what)) {
            (_, Ok(data)) => return Some(data),
            (true, Err(why)) => flash(page, &why),
            (false, Err(_)) => loading(page),
        }
        None
    }

    /// The data, or what to say instead (with the key that tries again),
    /// for a state shown inside a row. Disallowed by clippy.toml: `.ok()`
    /// would drop what to say, so each use says where it's said.
    pub fn text(self, what: &str) -> Result<&'a T, String> {
        let again = match self.retry {
            "" => String::new(),
            key => format!(". {key} tries again."),
        };
        match (self.data, self.error) {
            (Some(data), _) => Ok(data),
            (None, Some(err)) => Err(format!("Couldn't load {what}: {err}{again}")),
            (None, None) => Err(format!("Loading {what}…")),
        }
    }

    /// The data if it's ready, saying nothing otherwise: for decorations
    /// whose absence is fine (a count, a branch name). Disallowed by
    /// clippy.toml, so each use says why silence is right.
    pub fn ready_unchecked(self) -> Option<&'a T> {
        self.data
    }
}

#[cfg(test)]
mod tests {
    #![expect(clippy::disallowed_methods, reason = "tests each state")]

    use super::*;
    use crate::page::PageLine;

    impl<'a, T: ?Sized> Fetched<'a, T> {
        /// A fetch that failed, as pages' tests need one.
        pub(crate) fn failed(err: &'a str) -> Self {
            Self::new_unchecked(None, false, Some(err), "r")
        }
    }

    /// Each state shows as itself: a stale copy as the data, a retry in
    /// flight as loading, a failure with why.
    #[test]
    fn each_state_shows_as_itself() {
        let mut page = Page::new(80);
        let ready = Fetched::new_unchecked(Some(&7), true, Some("old"), "r");
        assert_eq!(ready.show(&mut page, "x"), Some(&7));
        // Alone on the page, loading is a skeleton box.
        let _ = Fetched::<u8>::new_unchecked(None, false, None, "r").show(&mut page, "x");
        let retrying = Fetched::<u8>::new_unchecked(None, true, Some("old"), "r");
        assert_eq!(retrying.show(&mut page, "runs"), None);
        let failed = Fetched::<u8>::new_unchecked(None, false, Some("timed out"), "r");
        assert_eq!(failed.show(&mut page, "runs"), None);
        let wrong = Fetched::ready(&7).pick(|_| None::<&u8>);
        assert_eq!(wrong.ready_unchecked(), None);
        let text: Vec<String> = page.lines.iter().map(PageLine::text).collect();
        assert!(
            text.first().is_some_and(|l| l.contains("Loading…")),
            "{text:?}"
        );
        assert!(text.iter().any(|l| l == "Loading runs…"), "{text:?}");
        let why = "Couldn't load runs: timed out. r tries again.";
        assert!(text.iter().any(|l| l.contains(why)), "{text:?}");
        let why = "Couldn't load it: the wrong kind of data";
        assert_eq!(wrong.text("it"), Err(why.to_owned()));
    }
}
