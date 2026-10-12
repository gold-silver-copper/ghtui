//! Data from GitHub: what's fetched, when, and what's still running.

use super::*;

/// Data that comes from GitHub: what we have (possibly cached), whether a
/// refresh is in flight, and the last refresh error.
#[derive(Debug)]
pub struct Remote<T> {
    pub data: Option<T>,
    pub loading: bool,
    pub error: Option<String>,
    /// When `data` was fetched, while it's a copy from the cache.
    pub cached_at: Option<u64>,
    /// When it was last asked for (Unix seconds), to know when it's old.
    pub asked_at: Option<u64>,
    /// The cursor a next page was asked after, while that page is on its way.
    pub(super) more: Option<String>,
    /// A write was made since the last fetch was asked for.
    pub(super) stale: bool,
    /// Pages were added with Load more: only asking refetches it.
    pub(super) extended: bool,
    /// Load more was asked for while the list reloaded: asked once it's in.
    pub(super) more_wanted: bool,
}

impl<T> Default for Remote<T> {
    fn default() -> Self {
        Self {
            data: None,
            loading: false,
            error: None,
            cached_at: None,
            asked_at: None,
            more: None,
            stale: false,
            extended: false,
            more_wanted: false,
        }
    }
}

impl<T> Remote<T> {
    /// What a page shows of a fetch: one that hasn't started is loading.
    #[expect(
        clippy::disallowed_methods,
        reason = "the one place a fetch's state becomes what a page shows"
    )]
    pub fn fetched<'a>(remote: Option<&'a Self>, retry: &'a str) -> Fetched<'a, T> {
        let (data, loading, error) = remote.map_or((None, false, None), |r| {
            (r.data.as_ref(), r.loading, r.error.as_deref())
        });
        Fetched::new_unchecked(data, loading, error, retry)
    }

    pub fn cached(cached: Option<crate::store::Cached<T>>) -> Self {
        Self {
            cached_at: cached.as_ref().map(|c| c.fetched_at),
            data: cached.map(|c| c.value),
            ..Self::default()
        }
    }

    /// Where the fetch is at, without the data.
    pub fn status(&self) -> Remote<()> {
        Remote {
            data: self.data.as_ref().map(|_| ()),
            loading: self.loading,
            error: self.error.clone(),
            cached_at: self.cached_at,
            asked_at: self.asked_at,
            more: self.more.clone(),
            stale: self.stale,
            extended: self.extended,
            more_wanted: self.more_wanted,
        }
    }

    /// Starts a fetch, unless one is running or (without `force`) the data
    /// is good: fetched here (not from the disk cache) since the last write,
    /// or extended, which a refetch would cut back to its first page.
    pub(super) fn begin(&mut self, force: bool) -> bool {
        let good = self.data.is_some() && self.error.is_none() && self.cached_at.is_none();
        if self.loading || (!force && (self.extended || good && !self.stale)) {
            return false;
        }
        self.loading = true;
        self.stale = false;
        true
    }

    pub(crate) fn finish(&mut self, result: Result<T, ApiError>) {
        self.loading = false;
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
                self.cached_at = None;
                // A next page asked for before this belongs to the old list.
                self.more = None;
                self.extended = false;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }
}

impl Remote<Data> {
    /// Asks for the next page, unless something is on its way or there is
    /// none: the cursor to ask after. Asked while the list reloads, it's
    /// asked again once the list is in.
    pub(crate) fn ask_more(&mut self) -> Option<String> {
        if self.loading {
            self.more_wanted = self.data.is_some();
            return None;
        }
        let data = (self.data.as_ref()).filter(|_| self.more.is_none())?;
        self.more = Some(data.next_cursor()?.to_owned());
        self.more.clone()
    }
}

impl State {
    /// What a page needs fetched: Home's are its sections'.
    pub fn needs(&self, route: &Route) -> Vec<Need> {
        match route {
            Route::Home => self.home_needs(),
            _ => browse::needs(route),
        }
    }

    /// Fetches a page's data. The repository header shared by a repo's
    /// pages is only fetched when missing; Home's sections, also when
    /// they're old.
    #[must_use]
    pub fn ensure_route(&mut self, route: &Route, force: bool) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        for need in self.needs(route) {
            let header = browse::is_header(route, &need);
            let aged = *route == Route::Home && self.aged(&need);
            cmds.extend(self.ensure(need, (force || aged) && !header));
        }
        cmds
    }

    #[must_use]
    pub fn ensure(&mut self, need: Need, force: bool) -> Vec<Cmd> {
        let now = (self.clock)();
        let (started, api) = match need {
            Need::Pr(pr) => (
                self.prs.entry(pr.clone()).or_default().begin(force),
                Api::FetchPr(pr),
            ),
            Need::Data(key) => {
                let remote = self.data.entry(key.clone()).or_default();
                // What was still running is checked again whenever it's
                // wanted (going back to it, switching to its tab).
                let running = remote.data.as_ref().is_some_and(Data::running);
                let cached = remote.data.is_none();
                let force = force || (running && !remote.extended);
                let started = remote.begin(force);
                if started {
                    remote.asked_at = Some(now);
                }
                (started, Api::Fetch { cached, key })
            }
        };
        started.then_some(Cmd::Api(api)).into_iter().collect()
    }

    /// Fetches needed for the visible screen. `force` refreshes data we
    /// already have.
    #[must_use]
    pub fn load_visible(&mut self, force: bool) -> Vec<Cmd> {
        let mut cmds = Vec::new();
        if self.viewer.is_none() && force {
            cmds.push(Cmd::Api(Api::FetchViewer));
        }
        match self.screen() {
            Screen::Page(p) => {
                let route = p.route.clone();
                cmds.extend(self.ensure_route(&route, force));
                // A Review tab's diff starts again, as Files changed's does.
                if force && let Some(pr) = browse::glanced(&route) {
                    self.diffs.remove(&DiffOf::Pr(pr.clone()));
                }
                // An issue or pull request list's open and closed counts
                // are the header's, so refreshing the list refreshes them.
                if force && let Route::Issues { repo, .. } | Route::Pulls { repo, .. } = &route {
                    let header = Need::Data(DataKey::Repo(repo.clone()));
                    cmds.extend(self.ensure(header, true));
                }
            }
            Screen::Diff(d) => {
                let of = d.of.clone();
                if let DiffOf::Pr(pr) = &of {
                    cmds.extend(self.ensure(Need::Pr(pr.clone()), force));
                }
                // Settling starts it (again); a PR's waits for the PR, which may have merged.
                if force {
                    self.diffs.remove(&of);
                }
            }
        }
        cmds
    }
}

impl State {
    /// What's on screen and still running (a run, a job, its log,
    /// checks, a pull request GitHub is still checking), to fetch again
    /// until it's done. A list extended with Load more isn't: fetching it
    /// again would cut it back to its first page.
    pub fn running_here(&self) -> Vec<Need> {
        let Screen::Page(p) = self.screen() else {
            return Vec::new();
        };
        self.needs(&p.route)
            .into_iter()
            .filter(|need| match need {
                Need::Data(key) => self
                    .data
                    .get(key)
                    .is_some_and(|r| !r.extended && r.data.as_ref().is_some_and(Data::running)),
                Need::Pr(pr) => (self.prs.get(pr))
                    .and_then(|r| r.data.as_ref())
                    .is_some_and(PrDetail::settling),
            })
            .collect()
    }

    /// What's loading on the visible screen, for the status bar.
    pub fn busy(&self) -> Option<String> {
        if let Some(waiting) = act::waiting(self).or_else(|| crate::writes::waiting(self)) {
            return Some(waiting);
        }
        match self.screen() {
            Screen::Page(p) if self.fetches(&p.route).any(|r| r.more.is_some()) => {
                Some("Loading more".into())
            }
            Screen::Page(p) if self.page_loading(&p.route) => {
                // Cached data on screen says how old it is.
                Some(match self.page_cached_at(&p.route) {
                    Some(at) => {
                        let ago = crate::ui::time::ago(at, (self.clock)());
                        format!("Refreshing · cached {ago}")
                    }
                    None => "Loading".to_owned(),
                })
            }
            Screen::Page(_) => None,
            Screen::Diff(screen) => match self.diffs.get(&screen.of) {
                Some(diff) => diff.status(),
                None => Some(format!("Loading {}", screen.of)),
            },
        }
    }
}
