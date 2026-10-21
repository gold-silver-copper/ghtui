//! Messages and keys: what each one does to the state.

use super::*;
use crate::route::{app, issues, runs, search};

/// Handles a message and settles the screen.
#[cfg(test)]
pub fn update(state: &mut State, msg: Msg) -> Cmds {
    let mut cmds = apply_msg(state, msg);
    cmds.extend(state.settle());
    cmds
}

/// Handles a message without settling the screen: the runtime handles a
/// burst of background results, then settles once.
pub fn apply_msg(state: &mut State, msg: Msg) -> Cmds {
    // Who's signed in, and what the writes' answers changed of the
    // features' own state, show on pages too.
    let shown = matches!(msg, Msg::Viewer(_) | Msg::Done(_));
    let data = shown || matches!(msg, Msg::Fetched { .. } | Msg::FetchedMore(..));
    // What's followed until GitHub shows it is checked when data comes
    // in. A page opened from cached data alone brings no data: its forms
    // still need filling in. With data it's checked anyway.
    let opened = if data { None } else { state.route().cloned() };
    if shown {
        state.shown_gen += 1;
    }
    // A write changes what GitHub shows, wherever it shows it.
    let wrote = match &msg {
        Msg::Done(done) => done.wrote(),
        // A queued write reached GitHub.
        Msg::Offline(crate::offline::Msg::Queue(crate::offline::queue::Told::Taken(_))) => true,
        _ => false,
    };
    if wrote {
        state.data.values_mut().for_each(|r| r.stale = true);
    }
    // What came in, and whether GitHub sent it just now (not the cache).
    let fetched = match &msg {
        Msg::Fetched {
            key,
            result,
            cached_at,
            ..
        } => Some((key.clone(), result.is_ok(), cached_at.is_none())),
        _ => None,
    };
    let mut cmds = handle(state, msg);
    if let Some((DataKey::Unread, _, true)) = fetched {
        cmds.extend(state.unread_polled());
    }
    // Any page's data, fresh from GitHub, for the page watchers, and the
    // conversation's comments for what's new in them.
    if let Some((key, true, true)) = &fetched {
        if let DataKey::PrActivity(pr) = key {
            crate::live::activity_loaded(state, pr);
            cmds.extend(crate::talk::want_lines(state, pr));
        }
        crate::live::page::loaded(state, key);
    }
    // A pull request's checks, for its diff's annotations.
    if let Some((DataKey::PrChecks(pr), true, _)) = &fetched {
        crate::workflows::checks_loaded(state, pr);
    }
    if data || state.route() != opened.as_ref() {
        cmds.extend(crate::follow::follow(state));
        cmds.extend(crate::features::follow(state));
    }
    cmds
}

/// Timers the screen needs after an update: a new notice expiring, the
/// spinner's next frame while something loads. The runtime calls this.
pub fn timers(state: &mut State) -> Cmds {
    let mut cmds = Cmds::from_iter(state.notices.timer(state.clock));
    if !state.spinning && state.busy().is_some() {
        state.spinning = true;
        cmds.push(Cmd::Timer(Timer::Spin, 90));
    }
    let awaiting = crate::follow::waiting(state).is_some();
    if !state.live && (!state.running_here().is_empty() || awaiting) {
        state.live = true;
        cmds.push(Cmd::Timer(Timer::Live, LIVE_MS));
    }
    cmds.extend(crate::live::schedule(state));
    // Home keeps itself fresh while it's on screen, and only then.
    if !state.home_timer && state.route() == Some(&Route::App(app::Home)) {
        state.home_timer = true;
        cmds.push(Cmd::Timer(Timer::Home, crate::home::FRESH_SECS * 1000));
    }
    cmds
}

/// How often what's running on screen is fetched again.
pub(super) const LIVE_MS: u64 = 5_000;

pub(super) fn handle(state: &mut State, msg: Msg) -> Cmds {
    match msg {
        Msg::Diff(of, msg) => return diff_screen::update(state, &of, msg),
        Msg::Done(done) => return done.apply(state),
        Msg::Key(key) => return on_key(state, key),
        Msg::Resize(w, h) => state.size = (w, h),
        Msg::Viewer(Ok(login)) => state.viewer = Some(login),
        Msg::Viewer(Err(err)) => tracing::warn!(%err, "could not fetch viewer"),
        Msg::Timer(Timer::Unread(poll)) => return state.unread_due(poll),
        Msg::Fetched {
            key,
            result,
            cached_at,
            source,
        } => {
            if let Err(err) = &result {
                tracing::warn!(?key, %err, "fetch failed");
            }
            let is_pr = matches!(result, Ok(Data::Issue(None)));
            let remote = state.data.entry(key.clone()).or_default();
            remote.source = source.or(remote.source.take());
            match cached_at {
                None => remote.finish(result),
                // A first look while it loads: shown over nothing, or over
                // an older one.
                Some(at) => {
                    let older = remote.data.is_none() || remote.cached_at.is_some_and(|c| c <= at);
                    if older && let Ok(data) = result {
                        remote.data = Some(data);
                        remote.cached_at = Some(at);
                        remote.changed();
                    }
                }
            }
            // A write landed while this was on its way, so it may miss it.
            if remote.stale && !is_pr {
                return state.ensure(key, false);
            }
            // Load more, asked while it reloaded.
            if !remote.loading
                && std::mem::take(&mut remote.more_wanted)
                && remote.error.is_none()
                && let Some(after) = remote.ask_more()
            {
                return Cmd::Api(Api::FetchMore { key, after }).into();
            }
            // GitHub redirects an issue number that's a pull request.
            if is_pr
                && let DataKey::Issue(repo, number) = key
                && state.route()
                    == Some(&Route::Issues(issues::Issue {
                        repo: repo.clone(),
                        number,
                    }))
            {
                let anchor = state.screen().page().and_then(|p| p.anchor.clone());
                return state.replace(Route::pr(PrRef { repo, number }), true, anchor);
            }
        }
        Msg::FetchedMore(key, after, result, source) => {
            let Some(remote) = (state.data.get_mut(&key)).filter(|r| r.more == Some(after)) else {
                return Cmds::new();
            };
            remote.more = None;
            remote.changed();
            remote.source = source.or(remote.source.take());
            match result {
                Ok(more) => {
                    if let Some(data) = &mut remote.data {
                        data.append(more);
                        remote.extended = true;
                    }
                }
                Err(err) => state.error(format!("Couldn't load more: {err}")),
            }
        }
        Msg::Mouse(ev) => return nav::on_mouse(state, ev),
        Msg::Timer(Timer::ExpireNotice(id)) => state.notices.expire(id),
        Msg::Timer(Timer::Spin) => {
            state.spinning = false;
            state.spinner = state.spinner.wrapping_add(1);
        }
        Msg::Timer(Timer::Live) => {
            state.live = false;
            let mut cmds = Cmds::new();
            let mut needs = state.running_here();
            needs.extend(crate::follow::poll(state));
            for need in needs {
                cmds.extend(state.ensure(need, true));
            }
            return cmds;
        }
        Msg::Timer(Timer::Home) => {
            state.home_timer = false;
            if state.route() == Some(&Route::App(app::Home)) {
                return state.ensure_route(&Route::App(app::Home), false);
            }
        }
        // Settling rebuilds the page if a time it shows has moved on.
        Msg::Timer(Timer::Minute) => {
            state.minute = (state.clock)();
            return Cmd::Timer(Timer::Minute, 60_000).into();
        }
        Msg::SuggestDue(q) => {
            if let Some(Overlay::Search(sb)) = &state.overlay
                && sb.query() == q
            {
                return state.ensure(DataKey::Suggest(q), false);
            }
        }
        Msg::RateLimits(limits) => state.rate_limits = limits,
        Msg::Notice(notice) => state.notices.say(notice),
        Msg::Problem(problem, Some(text)) => {
            state.problems.insert(problem, text);
        }
        Msg::Problem(problem, None) => {
            state.problems.remove(&problem);
        }
        Msg::Timer(Timer::Watch(id)) => return crate::live::on_timer(state, id),
        Msg::Polled(pr, answers) => return crate::live::on_polled(state, &pr, answers),
        Msg::Watched(what, answers) => return crate::live::page::on_watched(state, &what, answers),
        Msg::Paste(text) => return crate::typing::on_paste(state, &text),
        Msg::Focus(focused) => crate::live::on_focus(state, focused),
        Msg::Code(ask, result) => return crate::code_nav::answered(state, &ask, result),
        Msg::Edited(purpose, result) => return on_edited(state, purpose, result),
        Msg::Offline(msg) => return crate::offline::on_msg(state, msg),
        Msg::Timer(Timer::Probe) => return crate::offline::on_timer(state),
    }
    Cmds::new()
}

pub(super) fn on_key(state: &mut State, key: KeyEvent) -> Cmds {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.quit = crate::compose::may_discard(state, "ctrl-c", "quit and discard");
        return Cmds::new();
    }
    match &mut state.overlay {
        Some(Overlay::Picker(_)) => return picker::on_key(state, key),
        Some(Overlay::Find(_)) => return on_search_key(state, key),
        Some(Overlay::Search(_)) => return nav::on_search_box_key(state, key),
        Some(Overlay::Hints(_)) => return nav::on_hints_key(state, key),
        Some(Overlay::Menu(_)) => return nav::on_menu_key(state, key),
        Some(Overlay::Compose(_)) => return crate::compose::on_key(state, key),
        Some(Overlay::Submit(_)) => return on_submit_key(state, key),
        Some(Overlay::Ask(_)) => return crate::forms::on_ask_key(state, key),
        None => {}
    }
    // Any keypress dismisses the last notice; Esc on an error does only
    // that (the messages list keeps it).
    let dismissing = matches!(state.notices.dismiss(), Some(Notice::Error(_)));
    if dismissing && key.code == KeyCode::Esc {
        return Cmds::new();
    }
    let key = crate::keymap::Key::from(key);
    match state.keymap.resolve(key, state.contexts()) {
        Some(action) => apply(state, action),
        None => {
            let shown = crate::keymap::pretty(key);
            match state.keymap.elsewhere(key) {
                Some(action) => {
                    state.info(format!("{shown} does nothing here · {}", action.not_here()));
                }
                None => {
                    let menu = state.first_key(Action::Menu);
                    state.info(format!("{shown}: not bound here · {menu} shows what is"));
                }
            }
            Cmds::new()
        }
    }
}

pub(super) fn on_search_key(state: &mut State, key: KeyEvent) -> Cmds {
    let Some(Overlay::Find(input)) = &mut state.overlay else { return Cmds::new() };
    match key.code {
        KeyCode::Esc => state.overlay = None,
        KeyCode::Enter => {
            let query = input.lines().join("");
            state.overlay = None;
            if !query.is_empty() {
                match state.diff_parts() {
                    Some((screen, diff)) => {
                        let found = diff_screen::search(screen, diff, query, true);
                        state.notices.say(found);
                    }
                    None => nav::find(state, &query, true),
                }
            }
        }
        _ => nav::type_into(input, key),
    }
    Cmds::new()
}

/// Runs `action`. The actions every screen shares are here; the rest go to
/// the screen's own handler, which says where an action works when it
/// doesn't work there.
pub fn apply(state: &mut State, action: Action) -> Cmds {
    // On the diff: whether a search or a selection is active.
    let diff = (state.screen().diff()).map(|d| d.search.is_some() || d.selection.is_some());
    match action {
        Action::Quit => {
            state.quit = crate::compose::may_discard(state, "q", "quit and discard");
        }
        // Esc ends a search or selection; with neither, it goes back.
        Action::Cancel if diff == Some(true) => {
            if let Some(screen) = state.screen_mut().diff_mut() {
                (screen.search, screen.selection) = (None, None);
            }
            state.info("Cleared");
        }
        Action::Cancel if matches!(state.route(), Some(Route::Runs(runs::Job { query, .. })) if !query.is_empty()) =>
        {
            nav::search_log(state, "");
        }
        Action::Cancel => return nav::cancel(state),
        Action::Back => return state.back(),
        Action::Forward => return state.go_forward(),
        // From the files, up is the pull request.
        Action::UpLevel if diff.is_some() => return nav::switch_tab(state, 1),
        Action::UpLevel => return nav::up_level(state),
        Action::NextTab => return nav::step_tab(state, true),
        Action::PrevTab => return nav::step_tab(state, false),
        _ if let Some(n) = Action::PAGE_TABS.iter().position(|a| *a == action) => {
            return nav::switch_tab(state, n + 1);
        }
        Action::NextOpenTab => return state.step_open_tab(true),
        Action::PrevOpenTab => return state.step_open_tab(false),
        Action::OpenInTab => return state.open_in_tab(),
        Action::CloseTab => return state.close_tab(),
        _ if let Some(n) = Action::OPEN_TABS.iter().position(|a| *a == action) => {
            return state.switch_to_tab(n);
        }
        Action::GoHome if state.route() == Some(&Route::App(app::Home)) => {
            state.info("Already home");
        }
        Action::GoHome => return state.push(Route::App(app::Home)),
        Action::Search if diff.is_some() || !nav::searches_itself(state) => {
            let input = nav::new_input(&state.theme, Bg::Container);
            state.overlay = Some(Overlay::Find(Box::new(input)));
        }
        Action::Search => return state.open_search(false),
        Action::SearchWord => return nav::search_word(state),
        Action::OpenLink => return nav::open_link(state),
        Action::Symbols => return crate::code_nav::symbols(state),
        Action::Definition => return crate::code_nav::definition(state),
        Action::RepoSymbols => return crate::code_nav::repo_symbols(state),
        Action::References => return crate::code_nav::references(state),
        Action::Notifications => {
            let inbox = Route::Search(search::Notifications {
                query: String::new(),
            });
            if state.route() == Some(&inbox) {
                state.info("Already in your notifications");
            } else {
                return state.push(inbox);
            }
        }
        Action::ReviewGlance => return nav::review_glance(state),
        Action::Explore => {
            let explore = Route::Site(crate::site::Page::Explore {
                language: None,
                since: crate::api::explore::Since::default(),
            });
            return state.push(explore);
        }
        Action::SearchGithub => return state.open_search(true),
        Action::FindFile if diff.is_some() => return state.open_picker(picker::Kind::DiffFiles),
        Action::FindFile => return state.open_finder(false),
        Action::Menu => nav::open_menu(state),
        Action::CommandPalette => return state.open_picker(picker::Kind::Commands),
        Action::Messages => {
            let kept = state.notices.log().iter().cloned().collect();
            return state.open_picker(picker::Kind::Messages(kept));
        }
        // The diff's links are in its threads' comments.
        Action::Hints | Action::HintsBrowser if diff.is_some() => {
            nav::start_hints(state, action == Action::HintsBrowser);
        }
        // What a feature does: queued writes and refreshing while GitHub
        // can't be reached among them.
        _ if let Some(feature) = crate::features::of(state, action) => {
            return (feature.act)(state, action);
        }
        Action::Refresh => return state.load_visible(true),
        Action::Copy => return nav::copy_link(state),
        Action::OpenInBrowser => return state.go(Target::External(state.here_url()).into()),
        _ if diff.is_some() => return diff_action(state, action),
        _ => return nav::page_action(state, action),
    }
    Cmds::new()
}

/// Actions on the diff screen: reviewing, and the rest on the diff itself.
pub(super) fn diff_action(state: &mut State, action: Action) -> Cmds {
    let content = state.layout().content;
    let here = state.screen().diff().zip(state.diff());
    let on_annotation = here.is_some_and(|(screen, diff)| {
        let under = diff.doc.annotation_under(screen.cursor, screen.half);
        screen.focus == Pane::Diff && under.is_some()
    });
    match action {
        Action::Comment
        | Action::FileComment
        | Action::Suggest
        | Action::ResolveThread
        | Action::Delete
        | Action::UndoDelete
        | Action::SubmitReview
        | Action::ToggleSinceReview
        | Action::PickCommits => return review_action(state, action),
        Action::Open if on_annotation => return review_action(state, action),
        _ => {}
    }
    let State {
        screens,
        diffs,
        notices,
        ..
    } = &mut *state;
    let Screen::Diff(screen) = screens.last_mut() else { return Cmds::new() };
    match diffs.get_mut(&screen.of) {
        Some(diff) => diff_screen::apply(screen, diff, action, content, notices),
        None => {
            notices.info("The diff hasn't loaded yet");
            Cmds::new()
        }
    }
}
