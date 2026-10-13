//! Messages and keys: what each one does to the state.

use super::*;

/// Handles a message and settles the screen.
#[cfg(test)]
pub fn update(state: &mut State, msg: Msg) -> Vec<Cmd> {
    let mut cmds = apply_msg(state, msg);
    cmds.extend(state.settle());
    cmds
}

/// Handles a message without settling the screen: the runtime handles a
/// burst of background results, then settles once.
#[must_use]
pub fn apply_msg(state: &mut State, msg: Msg) -> Vec<Cmd> {
    // Pages are built from GitHub's data; only these change it.
    let data = matches!(
        msg,
        Msg::Viewer(_)
            | Msg::Pr(..)
            | Msg::Fetched { .. }
            | Msg::FetchedMore(..)
            | Msg::Source(..)
            | Msg::Changed(..)
            | Msg::Create(_)
            | Msg::Write(_)
    );
    if data {
        state.data_gen += 1;
    }
    // A write changes what GitHub shows, wherever it shows it.
    let wrote = match &msg {
        Msg::Changed(_, _, r) => r.is_ok(),
        Msg::Create(crate::create::Done::Created(_, r)) => r.is_ok(),
        Msg::Write(done) => done.wrote(),
        // Drafts GitHub accepted wait in a pending review there.
        Msg::Diff(_, DiffMsg::ReviewSubmitted(o)) => o.error.is_none() || !o.accepted.is_empty(),
        _ => false,
    };
    if wrote {
        state.prs.values_mut().for_each(|r| r.stale = true);
        state.data.values_mut().for_each(|r| r.stale = true);
    }
    // The conversation's comments, fresh from GitHub.
    let activity = match &msg {
        Msg::Fetched {
            key: DataKey::PrActivity(pr),
            result: Ok(_),
            cached_at: None,
        } => Some(pr.clone()),
        _ => None,
    };
    // A pull request's checks, for its diff's annotations.
    let checks = match &msg {
        Msg::Fetched {
            key: DataKey::PrChecks(pr),
            result: Ok(_),
            ..
        } => Some(pr.clone()),
        _ => None,
    };
    // Any page's data, fresh from GitHub, for the page watchers.
    let fresh = match &msg {
        Msg::Fetched {
            key,
            result: Ok(_),
            cached_at: None,
        } => Some(key.clone()),
        _ => None,
    };
    let mut cmds = handle(state, msg);
    if let Some(pr) = activity {
        crate::live::activity_loaded(state, &pr);
        cmds.extend(crate::talk::want_lines(state, &pr));
    }
    if let Some(key) = fresh {
        crate::live::page::loaded(state, &key);
    }
    if let Some(pr) = checks {
        crate::actions::checks_loaded(state, &pr);
    }
    if data {
        cmds.extend(act::follow(state));
        cmds.extend(crate::create::follow(state));
        cmds.extend(crate::writes::follow(state));
    }
    cmds
}

/// Timers the screen needs after an update: a new notice expiring, the
/// spinner's next frame while something loads. The runtime calls this.
#[must_use]
pub fn timers(state: &mut State) -> Vec<Cmd> {
    let mut cmds = Vec::from_iter(state.notices.timer(state.clock));
    if state.busy().is_some() && !state.spinning {
        state.spinning = true;
        cmds.push(Cmd::Timer(Timer::Spin, 90));
    }
    let awaiting = act::awaiting_here(state).is_some() || crate::writes::awaiting(state);
    if !state.live && (!state.running_here().is_empty() || awaiting) {
        state.live = true;
        cmds.push(Cmd::Timer(Timer::Live, LIVE_MS));
    }
    cmds.extend(crate::live::schedule(state));
    // Home keeps itself fresh while it's on screen, and only then.
    if !state.home_timer && state.route() == Some(&Route::Home) {
        state.home_timer = true;
        cmds.push(Cmd::Timer(Timer::Home, crate::home::FRESH_SECS * 1000));
    }
    cmds
}

/// How often what's running on screen is fetched again.
pub(super) const LIVE_MS: u64 = 5_000;

#[must_use]
pub(super) fn handle(state: &mut State, msg: Msg) -> Vec<Cmd> {
    match msg {
        Msg::Diff(of, msg) => return diff_screen::update(state, &of, msg),
        Msg::Create(done) => return crate::create::on_done(state, done),
        Msg::Write(done) => return crate::writes::on_done(state, done),
        Msg::Actions(done) => return crate::actions::on_done(state, done),
        Msg::Key(key) => return on_key(state, key),
        Msg::Resize(w, h) => state.size = (w, h),
        Msg::Viewer(Ok(login)) => state.viewer = Some(login),
        Msg::Viewer(Err(err)) => tracing::warn!(%err, "could not fetch viewer"),
        Msg::Unread(result) => return state.unread_polled(result),
        Msg::Timer(Timer::Unread(poll)) => return state.unread_due(poll),
        Msg::Pr(pr, result) => {
            if let Err(err) = result.as_ref() {
                tracing::warn!(%pr, %err, "PR fetch failed");
            }
            state.prs.entry(pr).or_default().finish(*result);
        }
        Msg::Fetched {
            key,
            result,
            cached_at,
        } => {
            if let Err(err) = &result {
                tracing::warn!(?key, %err, "fetch failed");
            }
            let is_pr = matches!(result, Ok(Data::Issue(None)));
            let remote = state.data.entry(key.clone()).or_default();
            match cached_at {
                None => remote.finish(result),
                // A first look while it loads: shown over nothing, or over
                // an older one.
                Some(at) => {
                    let older = remote.data.is_none() || remote.cached_at.is_some_and(|c| c <= at);
                    if older && let Ok(data) = result {
                        remote.data = Some(data);
                        remote.cached_at = Some(at);
                    }
                }
            }
            // A write landed while this was on its way, so it may miss it.
            if remote.stale && !is_pr {
                return state.ensure(Need::Data(key), false);
            }
            // Load more, asked while it reloaded.
            if !remote.loading
                && std::mem::take(&mut remote.more_wanted)
                && remote.error.is_none()
                && let Some(after) = remote.ask_more()
            {
                return vec![Cmd::Api(Api::FetchMore { key, after })];
            }
            // GitHub redirects an issue number that's a pull request.
            if is_pr
                && let DataKey::Issue(repo, number) = key
                && state.route()
                    == Some(&Route::Issue {
                        repo: repo.clone(),
                        number,
                    })
            {
                let anchor = match state.screen() {
                    Screen::Page(p) => p.anchor.clone(),
                    Screen::Diff(_) => None,
                };
                return state.replace(Route::pr(PrRef { repo, number }), true, anchor);
            }
        }
        Msg::Source(key, from) => {
            state.sources.insert(key, from);
        }
        Msg::FetchedMore(key, after, result) => {
            let Some(remote) = (state.data.get_mut(&key)).filter(|r| r.more == Some(after)) else {
                return Vec::new();
            };
            remote.more = None;
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
        Msg::Changed(change, by, result) => return act::on_changed(state, &change, by, result),
        Msg::Mouse(ev) => return nav::on_mouse(state, ev),
        Msg::Timer(Timer::ExpireNotice(id)) => state.notices.expire(id),
        Msg::Timer(Timer::Spin) => {
            state.spinning = false;
            state.spinner = state.spinner.wrapping_add(1);
        }
        Msg::Timer(Timer::Live) => {
            state.live = false;
            let mut cmds = Vec::new();
            let mut needs = state.running_here();
            needs.extend(act::poll(state));
            needs.extend(crate::writes::poll(state));
            for need in needs {
                cmds.extend(state.ensure(need, true));
            }
            return cmds;
        }
        Msg::Timer(Timer::Home) => {
            state.home_timer = false;
            if state.route() == Some(&Route::Home) {
                return state.ensure_route(&Route::Home, false);
            }
        }
        Msg::Timer(Timer::Minute) => {
            state.data_gen += 1;
            return vec![Cmd::Timer(Timer::Minute, 60_000)];
        }
        Msg::SuggestDue(q) => {
            if let Some(Overlay::Search(sb)) = &state.overlay
                && sb.query() == q
            {
                return vec![Cmd::Api(Api::Suggest(q))];
            }
        }
        Msg::Suggested(q, result) => {
            if let (Some(Overlay::Search(sb)), Ok(repos)) = (&mut state.overlay, result) {
                sb.suggested(q, repos);
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
        Msg::Mentionable(repo, result) => crate::typing::on_mentionable(state, repo, result),
        Msg::Code(ask, result) => return crate::code_nav::answered(state, &ask, result),
        Msg::Edited(purpose, result) => return on_edited(state, purpose, result),
        Msg::HomeEdited { edit, result, what } => return state.home_edited(&edit, result, what),
    }
    Vec::new()
}

#[must_use]
pub(super) fn on_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        state.quit = crate::review::may_discard(state, "ctrl-c", "quit and discard");
        return Vec::new();
    }
    match &mut state.overlay {
        Some(Overlay::Picker(_)) => return picker::on_key(state, key),
        Some(Overlay::Find(_)) => return on_search_key(state, key),
        Some(Overlay::Search(_)) => return nav::on_search_box_key(state, key),
        Some(Overlay::Hints(_)) => return nav::on_hints_key(state, key),
        Some(Overlay::Menu(_)) => return nav::on_menu_key(state, key),
        Some(Overlay::Compose(_)) => return on_compose_key(state, key),
        Some(Overlay::Submit(_)) => return on_submit_key(state, key),
        Some(Overlay::Confirm(_)) => return act::on_confirm_key(state, key),
        Some(Overlay::Ask(_)) => return crate::create::on_ask_key(state, key),
        None => {}
    }
    // Any keypress dismisses the last notice; Esc on an error does only
    // that (the messages list keeps it).
    let dismissing = matches!(state.notices.dismiss(), Some(Notice::Error(_)));
    if dismissing && key.code == KeyCode::Esc {
        (state.typed, state.count) = (Vec::new(), None);
        return Vec::new();
    }
    let key = Key::from(key);
    // Digits before a binding are a count.
    if let Some(digit) = key.digit().filter(|_| state.typed.is_empty())
        && (digit > 0 || state.count.is_some())
    {
        let count = state.count.unwrap_or(0).saturating_mul(10);
        state.count = Some(count.saturating_add(digit as usize).min(MAX_COUNT));
        return Vec::new();
    }
    state.typed.push(key);
    let resolved = state.keymap.resolve(&state.typed, state.contexts());
    if resolved == Resolved::Wait {
        return Vec::new();
    }
    let typed = std::mem::take(&mut state.typed);
    let count = state.count.take();
    match resolved {
        Resolved::Run(action) => run(state, action, count),
        // Esc gives up on keys typed so far.
        _ if typed.len() > 1 && key.code == KeyCode::Esc => Vec::new(),
        _ => {
            let shown = crate::keymap::pretty_keys(&typed);
            match state.keymap.elsewhere(&typed) {
                Some(action) => {
                    state.info(format!("{shown} does nothing here · {}", action.not_here()));
                }
                None => {
                    let menu = state.first_key(Action::Menu);
                    state.info(format!("{shown}: not bound here · {menu} shows what is"));
                }
            }
            Vec::new()
        }
    }
}

/// The largest count typed.
pub(super) const MAX_COUNT: usize = 9999;

/// Runs `action`, `count` times if it's a motion. With a count, top goes
/// to that line or row and the next tab is that tab.
#[must_use]
pub fn run(state: &mut State, action: Action, count: Option<usize>) -> Vec<Cmd> {
    match (action, count) {
        (Action::Top, Some(n)) => nav::go_to_line(state, n),
        (Action::NextTab, Some(n)) => nav::switch_tab(state, n),
        (action, Some(n)) if action.repeats() => {
            let mut cmds = Vec::new();
            for _ in 0..n {
                cmds.extend(apply(state, action));
            }
            cmds
        }
        (action, _) => apply(state, action),
    }
}

#[must_use]
pub(super) fn on_search_key(state: &mut State, key: KeyEvent) -> Vec<Cmd> {
    let Some(Overlay::Find(input)) = &mut state.overlay else {
        return Vec::new();
    };
    match key.code {
        KeyCode::Esc => state.overlay = None,
        KeyCode::Enter => {
            let query = input.lines().join("");
            state.overlay = None;
            let forward = !state.backwards;
            if !query.is_empty() {
                match state.diff_parts() {
                    Some((screen, diff)) => {
                        let found = diff_screen::search(screen, diff, query, forward);
                        state.notices.say(found);
                    }
                    None => nav::find(state, &query, forward),
                }
            }
        }
        _ => nav::type_into(input, key),
    }
    Vec::new()
}

/// Runs `action`. The actions every screen shares are here; the rest go to
/// the screen's own handler, which says where an action works when it
/// doesn't work there.
#[must_use]
pub fn apply(state: &mut State, action: Action) -> Vec<Cmd> {
    // On the diff: whether a search or a selection is active.
    let diff = match state.screen() {
        Screen::Diff(screen) => Some(screen.search.is_some() || screen.selection.is_some()),
        Screen::Page(_) => None,
    };
    match action {
        Action::Quit => state.quit = true,
        // After `?`, `n` goes up and `N` down.
        Action::SearchNext | Action::SearchPrev if state.backwards => {
            let flipped = if action == Action::SearchNext {
                Action::SearchPrev
            } else {
                Action::SearchNext
            };
            state.backwards = false;
            let cmds = apply(state, flipped);
            state.backwards = true;
            return cmds;
        }
        Action::CloseView if state.screens.len() > 1 => return state.back(),
        Action::CloseView if state.tab_count() > 1 => return state.close_tab(),
        Action::CloseView => {
            let quit = state.first_key(Action::Quit);
            state.info(format!("Nothing left to close · {quit} quits"));
        }
        // Esc ends a search or selection, and never leaves.
        Action::Cancel if diff == Some(true) => {
            if let Screen::Diff(screen) = state.screen_mut() {
                (screen.search, screen.selection) = (None, None);
            }
            state.info("Cleared");
        }
        Action::Cancel if matches!(state.route(), Some(Route::Job { query, .. }) if !query.is_empty()) =>
        {
            nav::search_log(state, "");
        }
        Action::Cancel => nav::cancel(state),
        Action::Back => return state.back(),
        Action::Forward => return state.go_forward(),
        // From the files, up is the pull request.
        Action::UpLevel if diff.is_some() => return nav::switch_tab(state, 1),
        Action::UpLevel => return nav::up_level(state),
        Action::NextTab => return nav::step_tab(state, true),
        Action::PrevTab => return nav::step_tab(state, false),
        _ if Action::PAGE_TABS.contains(&action) => {
            let n = Action::PAGE_TABS.iter().position(|a| *a == action);
            return nav::switch_tab(state, n.unwrap_or(0) + 1);
        }
        Action::NextOpenTab => return state.step_open_tab(true),
        Action::PrevOpenTab => return state.step_open_tab(false),
        Action::OpenInTab => return state.open_in_tab(),
        Action::CloseTab => return state.close_tab(),
        Action::OpenTab1
        | Action::OpenTab2
        | Action::OpenTab3
        | Action::OpenTab4
        | Action::OpenTab5
        | Action::OpenTab6
        | Action::OpenTab7
        | Action::OpenTab8
        | Action::OpenTab9 => {
            let n = Action::OPEN_TABS.iter().position(|a| *a == action);
            return state.switch_to_tab(n.unwrap_or(0));
        }
        Action::GoHome if state.route() == Some(&Route::Home) => state.info("Already home"),
        Action::GoHome => return state.push(Route::Home),
        Action::Search | Action::SearchBack if diff.is_some() || !nav::searches_itself(state) => {
            state.backwards = action == Action::SearchBack;
            let input = nav::new_input(&state.theme, Bg::Container);
            state.overlay = Some(Overlay::Find(Box::new(input)));
        }
        Action::Search | Action::SearchBack => {
            state.backwards = action == Action::SearchBack;
            return state.open_search(false);
        }
        Action::SearchWord | Action::SearchWordBack => {
            return nav::search_word(state, action == Action::SearchWord);
        }
        Action::OpenLink => return nav::open_link(state),
        Action::Symbols => return crate::code_nav::symbols(state),
        Action::Definition => return crate::code_nav::definition(state),
        Action::RepoSymbols => return crate::code_nav::repo_symbols(state),
        Action::References => return crate::code_nav::references(state),
        Action::Notifications => {
            let inbox = Route::Notifications {
                query: String::new(),
            };
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
        Action::Refresh => return state.load_visible(true),
        Action::Copy => return nav::copy_link(state),
        Action::OpenInBrowser => return state.go(Target::External(state.here_url()).into()),
        _ if crate::manage::claims(state, action) => return crate::manage::act(state, action),
        _ if act::ACTIONS.contains(&action) => return act::act(state, action),
        // The diff's links are in its threads' comments.
        Action::Hints | Action::HintsBrowser if diff.is_some() => {
            nav::start_hints(state, action == Action::HintsBrowser);
        }
        // A discussion's and a project's own ways to write and edit.
        _ if crate::discuss::takes(state, action) => return crate::discuss::act(state, action),
        _ if crate::projects::takes(state, action) => return crate::projects::act(state, action),
        _ if crate::writes::claims(state, action) => return crate::writes::act(state, action),
        _ if crate::edit::ACTIONS.contains(&action) => return crate::edit::act(state, action),
        Action::RunWorkflow => return crate::actions::action(state),
        Action::NewIssue
        | Action::NewPr
        | Action::Create
        | Action::PushBranch
        | Action::Checkout => return crate::create::action(state, action),
        _ if diff.is_some() => return diff_action(state, action),
        _ => return nav::page_action(state, action),
    }
    Vec::new()
}

/// Actions on the diff screen: reviewing, and the rest on the diff itself.
#[must_use]
pub(super) fn diff_action(state: &mut State, action: Action) -> Vec<Cmd> {
    let content = state.layout().content;
    let on_annotation = match (state.screen(), state.diff()) {
        (Screen::Diff(screen), Some(diff)) => {
            screen.focus == Pane::Diff
                && diff
                    .doc
                    .annotation_under(screen.cursor, screen.half)
                    .is_some()
        }
        _ => false,
    };
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
    let Screen::Diff(screen) = screens.last_mut() else {
        return Vec::new();
    };
    match diffs.get_mut(&screen.of) {
        Some(diff) => diff_screen::apply(screen, diff, action, content, notices),
        None => {
            notices.info("The diff hasn't loaded yet");
            Vec::new()
        }
    }
}
