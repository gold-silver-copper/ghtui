//! What you're notified of: subscribing to an issue's, a pull request's
//! or a discussion's thread, or ignoring it, and a repository's watch
//! settings, as GitHub's Subscribe and Watch buttons do. `,w` lists the
//! choices, with the one set now; the one chosen goes to GitHub, and the
//! subscription is read again until GitHub shows it.

use crate::api::GitHub;
use crate::api::model::NodeId;
use crate::api::watch::{Subscription, SubscriptionState, Watched};
use crate::ui::overlays::PaletteItem;

use crate::browse::DataKey;
use crate::keymap::Action;
use crate::nav::Doable;
use crate::picker::fuzzy_score;
use crate::route::{Dest, Route, Target};
use crate::state::{Cmd, Remote, Screen, State};
use crate::writes::{Chosen, Expected, Pick, Work as Writes, item, row};

/// Setting a subscription.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Work {
    pub of: Watched,
    pub id: NodeId,
    pub state: SubscriptionState,
}

#[derive(Debug)]
pub struct Done(pub Work, pub Result<(), String>);

impl Done {
    pub fn wrote(&self) -> bool {
        self.1.is_ok()
    }
}

impl Work {
    pub fn failed(&self, why: String) -> Done {
        Done(self.clone(), Err(why))
    }

    pub async fn run(self, gh: &GitHub) -> Done {
        let result = gh.subscribe(&self.of, &self.id, self.state).await;
        Done(self, result.map_err(|e| crate::writes::reason(&e)))
    }
}

/// What a page (or the row selected on a list) is about, to subscribe to.
fn of_route(route: &Route) -> Option<Watched> {
    Some(match route {
        Route::Issue { repo, number } => Watched::Thread(repo.clone(), *number),
        Route::Pr { pr, .. } => Watched::Thread(pr.repo.clone(), pr.number),
        Route::Discussion {
            of: crate::api::browse::DiscussionsOf::Repo(repo),
            number,
        } => Watched::Discussion(repo.clone(), *number),
        // An organization's discussion is in a repository its page
        // doesn't name.
        Route::Discussion { .. } | Route::Discussions { .. } => return None,
        route => Watched::Repo(route.repo()?.clone()),
    })
}

/// What `,w` is about here: the thread on screen, the one selected on a
/// list (your inbox, a search), or the repository.
pub fn subject(state: &State) -> Option<Watched> {
    match state.screen() {
        Screen::Diff(d) => {
            let pr = d.of.pr()?;
            Some(Watched::Thread(pr.repo.clone(), pr.number))
        }
        Screen::Page(p) => {
            let row = p.selected_link().and_then(crate::ui::page::Link::url);
            let row = row.and_then(|url| match Dest::from_url(url).target {
                Target::Page(
                    route @ (Route::Issue { .. } | Route::Pr { .. } | Route::Discussion { .. }),
                ) => of_route(&route),
                // A row that opens a pull request's files is about it too.
                Target::Files(crate::diff_screen::DiffOf::Pr(pr)) => {
                    Some(Watched::Thread(pr.repo, pr.number))
                }
                _ => None,
            });
            // A list's row is what it's about; a page is about itself.
            match of_route(&p.route) {
                Some(Watched::Repo(_)) | None => row.or_else(|| of_route(&p.route)),
                here => here,
            }
        }
    }
}

/// `,w`: the choices for what's here.
#[must_use]
pub fn open(state: &mut State) -> Vec<Cmd> {
    let Some(of) = subject(state) else {
        state.info("That works on an issue, a pull request, a discussion, or a repository's pages");
        return Vec::new();
    };
    // What GitHub has now, not what was read before.
    let mut cmds = state.ensure(
        crate::browse::Need::Data(DataKey::Subscription(of.clone())),
        true,
    );
    cmds.extend(crate::writes::open_picker(state, Pick::Watch(of)));
    cmds
}

/// The choices, with what each means, as GitHub words them.
fn choices(of: &Watched) -> [(SubscriptionState, &'static str, &'static str); 3] {
    use SubscriptionState as S;
    match of {
        Watched::Repo(_) => [
            (
                S::Unsubscribed,
                "Participating and @mentions",
                "only what you take part in or are mentioned in",
            ),
            (
                S::Subscribed,
                "All Activity",
                "every issue, pull request and discussion",
            ),
            (S::Ignored, "Ignore", "never, not even when mentioned"),
        ],
        _ => [
            (S::Subscribed, "Subscribe", "every update"),
            (
                S::Unsubscribed,
                "Unsubscribe",
                "only when you take part or are mentioned",
            ),
            (S::Ignored, "Ignore", "never, not even when mentioned"),
        ],
    }
}

/// The picker's rows: each choice, the current one marked, or why
/// there are none.
pub fn rows(state: &State, of: &Watched, q: &str) -> Vec<(PaletteItem, Option<Chosen>)> {
    let key = DataKey::Subscription(of.clone());
    #[expect(clippy::disallowed_methods, reason = "said as the picker's row")]
    let fetched = Remote::fetched(state.data.get(&key), "")
        .pick(<Subscription as crate::browse::Picked>::pick)
        .text("your subscription");
    let s = match fetched {
        Ok(s) => s,
        Err(why) => return vec![item(why, "", None)],
    };
    if !s.can {
        return vec![item("GitHub doesn't let you subscribe to it", "", None)];
    }
    let mut rows: Vec<_> = (choices(of).into_iter())
        .filter(|(_, label, _)| fuzzy_score(q, label).is_some())
        .map(|(set, label, means)| {
            let hint = if s.state == Some(set) {
                format!("current · {means}")
            } else {
                means.to_owned()
            };
            let work = Work {
                of: of.clone(),
                id: s.id.clone(),
                state: set,
            };
            item(label, hint, Some(Chosen::Subscribe(work)))
        })
        .collect();
    if let Watched::Repo(_) = of {
        rows.push(item(
            "Custom (releases, issues…): on github.com",
            "GitHub's API can't set it",
            None,
        ));
    }
    rows
}

/// What setting `state` did, as GitHub's button would then say it.
fn said(of: &Watched, state: SubscriptionState) -> String {
    use SubscriptionState as S;
    match (of, state) {
        (Watched::Repo(repo), S::Subscribed) => format!("Watching all of {repo}'s activity"),
        (Watched::Repo(repo), S::Unsubscribed) => {
            format!("Notified of {repo} when you take part or are mentioned")
        }
        (Watched::Repo(repo), S::Ignored) => format!("Ignoring {repo}"),
        (of, S::Subscribed) => format!("Subscribed to {of}"),
        (of, S::Unsubscribed) => format!("Unsubscribed from {of}"),
        (of, S::Ignored) => format!("Ignoring {of}"),
    }
}

#[must_use]
pub fn choose(state: &mut State, work: Work) -> Vec<Cmd> {
    state.info(format!("{}…", said(&work.of, work.state)));
    vec![Cmd::Write(Writes::Watch(work))]
}

/// GitHub's answer: the subscription is read again until it shows it.
#[must_use]
pub fn on_done(state: &mut State, Done(work, result): Done) -> Vec<Cmd> {
    match result {
        Ok(()) => {
            state.info(said(&work.of, work.state));
            if let Some(route) = state.route().cloned() {
                let what = Expected::Subscription(work.of.clone(), work.state);
                crate::writes::expect(state, route, what);
            }
            let key = DataKey::Subscription(work.of);
            let mut cmds = state.ensure(crate::browse::Need::Data(key), true);
            // What the page shows of it (a repository's watchers) follows.
            cmds.extend(state.load_visible(false));
            cmds
        }
        Err(why) => {
            // GitHub sets a thread's subscription only for a token with
            // the notifications scope, which gh doesn't ask for.
            let text = if why.contains("'notifications'") {
                "Subscribing to a thread takes your token's notifications scope: gh auth refresh -s notifications adds it".to_owned()
            } else if why.to_lowercase().contains("not accessible") {
                format!(
                    "GitHub didn't change your subscription: {why}. A fine-grained token can't: a classic one can"
                )
            } else {
                format!("GitHub didn't change your subscription: {why}")
            };
            state.error(text);
            Vec::new()
        }
    }
}

/// The menu's row, wherever there's something to subscribe to.
pub fn doable(state: &State) -> Option<Doable> {
    let of = subject(state)?;
    let label = match &of {
        Watched::Repo(_) => "Watch the repository, or ignore it",
        _ => "Subscribe, unsubscribe or ignore",
    };
    let s = state.picked::<Subscription>(&DataKey::Subscription(of));
    let why = s
        .filter(|s| !s.can)
        .map(|_| "GitHub doesn't let you subscribe to it".to_owned());
    Some(row("Change", Action::Watch, label, "", why))
}
