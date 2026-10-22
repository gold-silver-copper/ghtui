//! Following people, and your own status, as a profile's Follow button and
//! the website's avatar menu do. `:follow` on a profile follows them (or an
//! organization), or unfollows; `:set status` writes your status in a
//! picker (an emoji and a message), busy or not, and when it clears, or
//! clears it. Each goes to GitHub as a job, and the profile is read again
//! until GitHub shows it.

use crate::api::browse::Profile;
use crate::api::model::NodeId;
use crate::api::social::NewStatus;
use crate::route::{Route, people};

use crate::act::Step;
use crate::data::DataKey;
use crate::follow::{Expectation, Shown};
use crate::keymap::Action;
use crate::nav::{Doable, row};
use crate::picker::{Choice, Kind, Rows, entry, listed};
use crate::state::{Cmds, State};
use crate::work::{Cx, Failure, Job};

/// The actions this adds.
const ACTIONS: [Action; 2] = [Action::Follow, Action::SetStatus];

/// What following and your status add: their actions, on profiles.
pub const FEATURE: crate::features::Feature = crate::features::Feature {
    owns: &ACTIONS,
    act,
    offers,
    ..crate::features::Feature::NONE
};

/// The longest status message GitHub takes.
const MESSAGE_MAX: usize = 80;

/// When a status clears itself, as the avatar menu offers (GitHub's
/// "today" and "this week" are your time zone's, which ghtui doesn't
/// know: a day and a week stand in).
const CLEARS: [(&str, Option<u64>); 6] = [
    ("doesn't clear", None),
    ("clears in 30 minutes", Some(30 * 60)),
    ("clears in an hour", Some(3600)),
    ("clears in 4 hours", Some(4 * 3600)),
    ("clears in a day", Some(86_400)),
    ("clears in a week", Some(7 * 86_400)),
];

/// Following someone, or no longer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Follow {
    pub login: String,
    pub id: NodeId,
    pub org: bool,
    pub on: bool,
}

impl Job for Follow {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        Ok(cx.gh.follow(&self.id, self.org, self.on).await?)
    }

    /// GitHub's answer: the profile is read again until it shows it.
    fn done(self, state: &mut State, result: Result<(), Failure>) -> Cmds {
        let key = profile_key(&self.login);
        match result {
            Ok(()) => {
                let did = if self.on { "Following" } else { "Unfollowed" };
                state.info(format!("{did} @{}", self.login));
                let shows = Followed {
                    key: key.clone(),
                    on: self.on,
                };
                expect(state, shows);
                state.ensure(key, true)
            }
            Err(why) => {
                let what = if self.on {
                    "following them"
                } else {
                    "unfollowing them"
                };
                let fine = "Followers: write";
                state.error(refused(what, &why, "Following people", "user:follow", fine));
                Cmds::new()
            }
        }
    }
}

/// Setting your status, or clearing it (`None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetStatus {
    /// Your login.
    pub login: String,
    pub status: Option<NewStatus>,
}

impl Job for SetStatus {
    type Out = ();

    async fn run(&self, cx: &Cx) -> Result<(), Failure> {
        Ok(cx.gh.set_status(self.status.as_ref()).await?)
    }

    fn done(self, state: &mut State, result: Result<(), Failure>) -> Cmds {
        let key = profile_key(&self.login);
        match result {
            Ok(()) => {
                let said = match &self.status {
                    Some(_) => "Status set",
                    None => "Status cleared",
                };
                state.info(said);
                expect(
                    state,
                    StatusShown {
                        key: key.clone(),
                        status: self.status,
                    },
                );
                state.ensure(key, true)
            }
            Err(why) => {
                let what = if self.status.is_some() {
                    "the status"
                } else {
                    "clearing the status"
                };
                let fine = "a classic token's user scope (a fine-grained one can't)";
                state.error(refused(what, &why, "Setting your status", "user", fine));
                Cmds::new()
            }
        }
    }
}

/// Waits, on the page on screen, for GitHub to show `what`.
fn expect(state: &mut State, what: impl Expectation) {
    if let Some(route) = state.route().cloned() {
        crate::follow::expect(state, route, what);
    }
}

/// Why GitHub refused `what`: the scope `doing` takes, where GitHub says
/// the token lacks it, and what a fine-grained token needs.
fn refused(what: &str, why: &Failure, doing: &str, scope: &str, fine: &str) -> String {
    let why = why.to_string();
    if why.contains("required scopes") {
        return crate::api::scope_missing(doing, scope);
    }
    let lower = why.to_lowercase();
    if lower.contains("not accessible") || lower.contains("forbidden") {
        return format!(
            "GitHub refused {what}: {why}. Your token may lack the permission: a fine-grained token needs {fine}; a classic one, the {scope} scope"
        );
    }
    format!("GitHub refused {what}: {why}")
}

/// Following someone, once their profile shows it.
#[derive(Debug)]
struct Followed {
    key: DataKey,
    on: bool,
}

impl Expectation for Followed {
    fn needs(&self, _: &State, _: &Route) -> Vec<DataKey> {
        vec![self.key.clone()]
    }

    fn waiting(&self) -> &'static str {
        "Waiting for GitHub to show the follow"
    }

    fn shown(&self, state: &State, _: &Route) -> Shown {
        let p = state.picked::<Profile>(&self.key);
        p.is_some_and(|p| p.followed == Some(self.on)).into()
    }
}

/// Your status, once your profile shows it.
#[derive(Debug)]
struct StatusShown {
    key: DataKey,
    status: Option<NewStatus>,
}

impl Expectation for StatusShown {
    fn needs(&self, _: &State, _: &Route) -> Vec<DataKey> {
        vec![self.key.clone()]
    }

    fn waiting(&self) -> &'static str {
        "Waiting for GitHub to show your status"
    }

    /// Its message and whether you're busy: GitHub may write the emoji
    /// its own way.
    fn shown(&self, state: &State, _: &Route) -> Shown {
        let Some(p) = state.picked::<Profile>(&self.key) else { return Shown::No };
        let now = p.status.as_ref();
        let message = now.and_then(|s| s.message.as_deref()).unwrap_or_default();
        let busy = now.is_some_and(|s| s.busy);
        match &self.status {
            Some(s) => message == s.message.trim() && busy == s.busy,
            None => message.is_empty() && !busy,
        }
        .into()
    }
}

/// Where a profile is kept.
fn profile_key(login: &str) -> DataKey {
    DataKey::Profile(login.to_lowercase())
}

/// The profile on screen, by login, and as loaded.
fn profile_here(state: &State) -> Option<(&str, Option<&Profile>)> {
    let Some(Route::People(people::User { login, .. })) = state.route() else {
        return None;
    };
    Some((login, state.picked::<Profile>(&profile_key(login))))
}

/// What `:follow` does here.
fn plan_follow(state: &State) -> Result<Step, String> {
    let (login, p) = profile_here(state).ok_or("Following works on a profile")?;
    if is_you(state, login) {
        return Err("That's you: you can't follow yourself".into());
    }
    let p = p.ok_or("It's still loading")?;
    let (Some(id), Some(followed)) = (&p.id, p.followed) else {
        return Err("GitHub doesn't say whether you follow it".into());
    };
    let job = Follow {
        login: p.login.clone(),
        id: id.clone(),
        org: p.is_org,
        on: !followed,
    };
    let doing = if followed { "Unfollowing" } else { "Following" };
    Ok(Step::Start(format!("{doing} @{login}…"), job.into()))
}

/// What a status picker is writing: whether it's busy, once chosen, and
/// what was typed, once it's opened again.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Composing {
    pub busy: Option<bool>,
    pub typed: Option<String>,
}

impl Composing {
    /// The field's text when it opens: what was typed, else your status.
    pub fn start(&self, state: &State) -> Option<String> {
        self.typed.clone().or_else(|| {
            let viewer = state.viewer.as_deref()?;
            let p = state.picked::<Profile>(&profile_key(viewer))?;
            p.status.as_ref()?.text()
        })
    }
}

/// What a status picker reads: your profile.
pub fn need(state: &State) -> Option<DataKey> {
    state.viewer.as_deref().map(profile_key)
}

fn act(state: &mut State, action: Action) -> Cmds {
    let planned = match action {
        Action::Follow => plan_follow(state),
        _ => match &state.viewer {
            Some(_) => Ok(Step::Pick(Kind::Status(Composing::default()))),
            None => Err("GitHub hasn't said who you are yet: try again in a moment".into()),
        },
    };
    crate::act::take(state, planned)
}

/// Whether the profile of `login` is yours.
fn is_you(state: &State, login: &str) -> bool {
    let viewer = state.viewer.as_deref();
    let p = state.picked::<Profile>(&profile_key(login));
    p.is_some_and(|p| p.is_viewer) || viewer.is_some_and(|v| v.eq_ignore_ascii_case(login))
}

fn offers(state: &State, out: &mut Vec<Doable>) {
    let Some((login, p)) = profile_here(state) else { return };
    if is_you(state, login) {
        let label = "Set your status, or clear it";
        out.push(row("Change", Action::SetStatus, label, "", None));
        return;
    }
    let label = match p.and_then(|p| p.followed) {
        Some(true) => format!("Unfollow @{login}"),
        _ => format!("Follow @{login}"),
    };
    let why = plan_follow(state).err();
    out.push(row("Change", Action::Follow, label, "", why));
}

/// An emoji and a message, as typed: the first word is the emoji when
/// it's a glyph or a `:name:`.
fn parse(typed: &str) -> (Option<String>, String) {
    let typed = typed.trim();
    let (first, rest) = typed.split_once(' ').unwrap_or((typed, ""));
    let named = first.len() > 2 && first.starts_with(':') && first.ends_with(':');
    let glyph = !first.is_ascii() && !first.chars().any(char::is_alphanumeric);
    if named || glyph {
        (Some(first.to_owned()), rest.trim().to_owned())
    } else {
        (None, typed.to_owned())
    }
}

/// The status picker's rows: setting what's typed, for each time it may
/// clear; busy or not; and clearing it.
pub fn rows(state: &State, c: &Composing, q: &str) -> Rows {
    let Some(viewer) = state.viewer.as_deref() else {
        return vec![entry("GitHub hasn't said who you are yet", "", None)];
    };
    let p = or_return!(listed::<Profile, _>(
        state,
        &profile_key(viewer),
        "your status"
    ));
    let now = p.status.as_ref();
    let busy = c.busy.unwrap_or_else(|| now.is_some_and(|s| s.busy));
    let (emoji, message) = parse(q);
    let mut rows = Vec::new();
    if message.chars().count() > MESSAGE_MAX {
        let long = format!(
            "GitHub takes a message of up to {MESSAGE_MAX} characters: this is {}",
            message.chars().count()
        );
        rows.push(entry(long, "", None));
    } else if q.is_empty() {
        rows.push(entry(
            "Type an emoji and a message: 🌴 On vacation",
            "",
            None,
        ));
    } else {
        let clock = (state.clock)();
        let shown = if busy {
            format!("{q} · Busy")
        } else {
            q.to_owned()
        };
        for (clears, after) in CLEARS {
            let status = NewStatus {
                emoji: emoji.clone(),
                message: message.clone(),
                busy,
                expires_at: after.map(|secs| crate::ui::time::iso8601(clock.saturating_add(secs))),
            };
            let job = SetStatus {
                login: viewer.to_owned(),
                status: Some(status),
            };
            let step = Step::Start("Setting your status…".into(), job.into());
            rows.push(entry(
                format!("Set “{shown}”"),
                clears,
                Some(Choice::Step(step)),
            ));
        }
    }
    let (mark, toggle) = if busy {
        ("✓", "↵ turns it off")
    } else {
        (" ", "↵ turns it on")
    };
    let again = Composing {
        busy: Some(!busy),
        typed: Some(q.to_owned()),
    };
    let step = Step::Pick(Kind::Status(again));
    rows.push(entry(
        format!("{mark} Busy: you may be slow to answer"),
        toggle,
        Some(Choice::Step(step)),
    ));
    if now.is_some() {
        let job = SetStatus {
            login: viewer.to_owned(),
            status: None,
        };
        let step = Step::Start("Clearing your status…".into(), job.into());
        rows.push(entry("Clear your status", "", Some(Choice::Step(step))));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first word is the emoji only when it's one.
    #[test]
    fn an_emoji_leads_the_message() {
        let words = |q: &str| parse(q);
        assert_eq!(
            words("🌴 On vacation"),
            (Some("🌴".into()), "On vacation".into())
        );
        assert_eq!(
            words(":palm_tree: Away"),
            (Some(":palm_tree:".into()), "Away".into())
        );
        assert_eq!(words("Away for a bit"), (None, "Away for a bit".into()));
        assert_eq!(words("Café open"), (None, "Café open".into()));
    }
}
