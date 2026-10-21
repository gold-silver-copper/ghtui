//! The diff on screen: starting it, settling it, and composing on it.

use super::*;

impl State {
    /// Opens a diff at `anchor` (a PR's starts once its metadata is in).
    pub fn open_diff(&mut self, of: DiffOf, anchor: Option<String>) -> Cmds {
        let (screen, cmds) = self.diff_screen(of, anchor);
        self.screens.push(screen);
        cmds
    }

    /// A screen of `of`'s files at `anchor`, its diff on its way.
    pub fn diff_screen(&mut self, of: DiffOf, anchor: Option<String>) -> (Screen, Cmds) {
        let mut cmds = match &of {
            DiffOf::Pr(pr) => self.ensure(DataKey::Pr(pr.clone()), false),
            DiffOf::Commit(..) | DiffOf::Range(..) => Cmds::new(),
        };
        if !self.diffs.get(&of).is_some_and(|d| d.error.is_none()) {
            cmds.extend(self.start_diff(&of));
        }
        let mut screen = DiffScreen::new(of, DiffPrefs::fit(self.size.0));
        screen.anchor = anchor;
        (Screen::Diff(Box::new(screen)), cmds)
    }

    /// The branch the PR merges into, once the PR has loaded.
    /// What the PR's diff is from, once the PR has loaded: its base
    /// branch, and for a merged or closed PR the base it had then (with a
    /// merge commit, its head is in the branch, which would leave nothing).
    pub(crate) fn pr_base(&self, pr: &PrRef) -> Option<crate::diff_job::PrBase> {
        let d = self.pr(pr)?;
        Some(crate::diff_job::PrBase {
            branch: d.base_ref.clone(),
            oid: (!d.state.is_open())
                .then(|| Oid::parse(&d.base_oid).ok())
                .flatten(),
        })
    }

    pub(super) fn start_diff(&mut self, of: &DiffOf) -> Cmds {
        let base = match of {
            DiffOf::Pr(pr) => match self.pr_base(pr) {
                Some(base) => base,
                None => return Cmds::new(),
            },
            DiffOf::Commit(..) | DiffOf::Range(..) => crate::diff_job::PrBase::default(),
        };
        let diff = DiffState::fresh();
        let job = diff.job;
        self.diffs.insert(of.clone(), diff);
        let mut cmds = Cmds::from(Cmd::Git(Git::LoadDiff {
            of: of.clone(),
            job,
            base,
            range: None,
        }));
        // Reviews, threads and viewed files are a PR's.
        if let DiffOf::Pr(pr) = of {
            cmds.extend([
                Cmd::Api(Api::FetchViewed(pr.clone())),
                Cmd::LoadReview(pr.clone()),
                Cmd::Api(Api::FetchThreads(pr.clone())),
                Cmd::Api(Api::FetchPatches(pr.clone())),
            ]);
            // Its checks' annotations show on their lines.
            cmds.extend(self.ensure(DataKey::PrChecks(pr.clone()), false));
            crate::workflows::checks_loaded(self, pr);
        }
        // A commit's comments on its lines, which join puts in its diff.
        if let DiffOf::Commit(repo, oid) = of {
            let key = DataKey::CommitComments(repo.clone(), oid.to_string());
            cmds.extend(self.ensure(key, false));
        }
        cmds
    }
}

impl State {
    /// Starts the diff on screen once it can and the joined work that's
    /// due, and re-runs the diff screen's clamping and prioritization.
    pub fn settle_diff(&mut self) -> Cmds {
        // A pull request's Review tab shows its diff's threads and drafts.
        let of = match self.screen() {
            Screen::Diff(screen) => Some(screen.of.clone()),
            Screen::Page(p) => browse::glanced(&p.route).cloned().map(DiffOf::Pr),
        };
        let mut cmds = match of {
            Some(of)
                if !self.diffs.contains_key(&of)
                    && (of.pr()).is_none_or(|pr| self.pr_remote(pr).is_none_or(|r| !r.loading)) =>
            {
                self.start_diff(&of)
            }
            _ => Cmds::new(),
        };
        cmds.extend(join::join(self));
        let content = self.layout().content;
        if let Some((screen, diff)) = self.diff_parts() {
            cmds.extend(diff_screen::settle(screen, diff, content));
        }
        cmds
    }
}

impl State {
    /// Opens the composer on `target`, starting with `text`.
    pub fn compose(&mut self, target: compose::Target, text: &str) -> Cmds {
        let compose = Compose::new(&self.theme, target, text);
        self.overlay = Some(Overlay::Compose(Box::new(compose)));
        crate::typing::fetch_mentionable(self)
    }

    /// The diff on screen, if it's one.
    pub fn diff(&self) -> Option<&DiffState> {
        match self.screen() {
            Screen::Diff(screen) => self.diffs.get(&screen.of),
            Screen::Page(_) => None,
        }
    }

    pub(crate) fn diff_parts(&mut self) -> Option<(&mut DiffScreen, &mut DiffState)> {
        let State { screens, diffs, .. } = self;
        let Screen::Diff(screen) = screens.last_mut() else {
            return None;
        };
        let diff = diffs.get_mut(&screen.of)?;
        Some((screen, diff))
    }
}
