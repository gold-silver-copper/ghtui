//! Rebindable keymap with multi-key sequences in vim notation:
//! `j`, `G`, `gg`, `]h`, `<C-d>`, `<Enter>`, `<Esc>`, `<Tab>`, `<Down>`.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Down,
    Up,
    HalfPageDown,
    HalfPageUp,
    Top,
    Bottom,
    Open,
    Back,
    Close,
    Quit,
    Refresh,
    OpenInBrowser,
    CommandPalette,
    Help,
    NextHunk,
    PrevHunk,
    NextFile,
    PrevFile,
    ToggleTree,
    SwitchPane,
    ToggleSplit,
    IgnoreWhitespace,
    ExpandContext,
    FullFile,
    ToggleViewed,
    NextUnviewed,
    MarkReviewed,
    Search,
    SearchNext,
    SearchPrev,
    FindFile,
    Comment,
    VisualLines,
    Suggest,
    ReplyThread,
    ResolveThread,
    DeleteDraft,
    FileComment,
    SubmitReview,
    NextThread,
    PrevThread,
    ToggleSinceReview,
    JumpMove,
    PickCommits,
    NextTab,
    PrevTab,
    GoHome,
    GoCode,
    GoIssues,
    GoPulls,
    Tab1,
    Tab2,
    Tab3,
    Tab4,
    Star,
    Hints,
    HintsBrowser,
    Forward,
    Copy,
    Menu,
    Branch,
    ToggleState,
    Sort,
    ScrollDown,
    ScrollUp,
    PageDown,
    PageUp,
    UpLevel,
}

/// Where a binding applies. Diff and page bindings may share keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Diff,
    Page,
}

impl Scope {
    fn overlaps(self, other: Scope) -> bool {
        self == Scope::Global || other == Scope::Global || self == other
    }
}

impl Action {
    pub const ALL: [Action; 68] = [
        Action::Down,
        Action::Up,
        Action::HalfPageDown,
        Action::HalfPageUp,
        Action::Top,
        Action::Bottom,
        Action::Open,
        Action::Back,
        Action::Close,
        Action::Quit,
        Action::Refresh,
        Action::OpenInBrowser,
        Action::CommandPalette,
        Action::Help,
        Action::NextHunk,
        Action::PrevHunk,
        Action::NextFile,
        Action::PrevFile,
        Action::ToggleTree,
        Action::SwitchPane,
        Action::ToggleSplit,
        Action::IgnoreWhitespace,
        Action::ExpandContext,
        Action::FullFile,
        Action::ToggleViewed,
        Action::NextUnviewed,
        Action::MarkReviewed,
        Action::Search,
        Action::SearchNext,
        Action::SearchPrev,
        Action::FindFile,
        Action::Comment,
        Action::VisualLines,
        Action::Suggest,
        Action::ReplyThread,
        Action::ResolveThread,
        Action::DeleteDraft,
        Action::FileComment,
        Action::SubmitReview,
        Action::NextThread,
        Action::PrevThread,
        Action::ToggleSinceReview,
        Action::JumpMove,
        Action::PickCommits,
        Action::NextTab,
        Action::PrevTab,
        Action::GoHome,
        Action::GoCode,
        Action::GoIssues,
        Action::GoPulls,
        Action::Tab1,
        Action::Tab2,
        Action::Tab3,
        Action::Tab4,
        Action::Star,
        Action::Hints,
        Action::HintsBrowser,
        Action::Forward,
        Action::Copy,
        Action::Menu,
        Action::Branch,
        Action::ToggleState,
        Action::Sort,
        Action::ScrollDown,
        Action::ScrollUp,
        Action::PageDown,
        Action::PageUp,
        Action::UpLevel,
    ];

    pub fn scope(self) -> Scope {
        match self {
            Action::Down
            | Action::Up
            | Action::HalfPageDown
            | Action::HalfPageUp
            | Action::Top
            | Action::Bottom
            | Action::Open
            | Action::Back
            | Action::Close
            | Action::Quit
            | Action::Refresh
            | Action::OpenInBrowser
            | Action::CommandPalette
            | Action::Help
            | Action::Search
            | Action::Comment
            | Action::FindFile
            | Action::GoHome
            | Action::GoIssues
            | Action::GoPulls
            | Action::Tab1
            | Action::Tab2
            | Action::Tab3
            | Action::Tab4
            | Action::Forward
            | Action::Copy
            | Action::Menu
            | Action::NextTab
            | Action::PrevTab
            | Action::PageDown
            | Action::PageUp
            | Action::UpLevel => Scope::Global,
            Action::GoCode
            | Action::Star
            | Action::Hints
            | Action::HintsBrowser
            | Action::Branch
            | Action::ToggleState
            | Action::Sort
            | Action::ScrollDown
            | Action::ScrollUp => Scope::Page,
            _ => Scope::Diff,
        }
    }

    /// The name used in the `[keys]` config table.
    pub fn name(self) -> &'static str {
        match self {
            Action::Down => "down",
            Action::Up => "up",
            Action::HalfPageDown => "half_page_down",
            Action::HalfPageUp => "half_page_up",
            Action::Top => "top",
            Action::Bottom => "bottom",
            Action::Open => "open",
            Action::Back => "back",
            Action::Close => "close",
            Action::Quit => "quit",
            Action::Refresh => "refresh",
            Action::OpenInBrowser => "open_in_browser",
            Action::CommandPalette => "command_palette",
            Action::Help => "help",
            Action::NextHunk => "next_hunk",
            Action::PrevHunk => "prev_hunk",
            Action::NextFile => "next_file",
            Action::PrevFile => "prev_file",
            Action::ToggleTree => "toggle_tree",
            Action::SwitchPane => "switch_pane",
            Action::ToggleSplit => "toggle_split",
            Action::IgnoreWhitespace => "ignore_whitespace",
            Action::ExpandContext => "expand_context",
            Action::FullFile => "full_file",
            Action::ToggleViewed => "toggle_viewed",
            Action::NextUnviewed => "next_unviewed",
            Action::MarkReviewed => "mark_reviewed",
            Action::Search => "search",
            Action::SearchNext => "search_next",
            Action::SearchPrev => "search_prev",
            Action::FindFile => "find_file",
            Action::Comment => "comment",
            Action::VisualLines => "visual_lines",
            Action::Suggest => "suggest",
            Action::ReplyThread => "reply",
            Action::ResolveThread => "resolve",
            Action::DeleteDraft => "delete_draft",
            Action::FileComment => "file_comment",
            Action::SubmitReview => "submit_review",
            Action::NextThread => "next_thread",
            Action::PrevThread => "prev_thread",
            Action::ToggleSinceReview => "since_review",
            Action::JumpMove => "jump_move",
            Action::PickCommits => "pick_commits",
            Action::NextTab => "next_tab",
            Action::PrevTab => "prev_tab",
            Action::GoHome => "go_home",
            Action::GoCode => "go_code",
            Action::GoIssues => "go_issues",
            Action::GoPulls => "go_pulls",
            Action::Tab1 => "tab_1",
            Action::Tab2 => "tab_2",
            Action::Tab3 => "tab_3",
            Action::Tab4 => "tab_4",
            Action::Star => "star",
            Action::Hints => "hints",
            Action::HintsBrowser => "hints_browser",
            Action::Forward => "forward",
            Action::Copy => "copy_link",
            Action::Menu => "actions_menu",
            Action::Branch => "branch",
            Action::ToggleState => "toggle_state",
            Action::Sort => "sort",
            Action::ScrollDown => "scroll_down",
            Action::ScrollUp => "scroll_up",
            Action::PageDown => "page_down",
            Action::PageUp => "page_up",
            Action::UpLevel => "up_level",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Action::Down => "Move down",
            Action::Up => "Move up",
            Action::HalfPageDown => "Half page down",
            Action::HalfPageUp => "Half page up",
            Action::Top => "Go to top",
            Action::Bottom => "Go to bottom",
            Action::Open => "Open the selection",
            Action::Back => "Back",
            Action::Close => "Close the page",
            Action::Quit => "Quit",
            Action::Refresh => "Refresh",
            Action::OpenInBrowser => "Open on GitHub",
            Action::CommandPalette => "Command palette",
            Action::Help => "Keyboard shortcuts",
            Action::NextHunk => "Next hunk",
            Action::PrevHunk => "Previous hunk",
            Action::NextFile => "Next file",
            Action::PrevFile => "Previous file",
            Action::ToggleTree => "Toggle file tree",
            Action::SwitchPane => "Switch pane",
            Action::ToggleSplit => "Split or unified view",
            Action::IgnoreWhitespace => "Ignore whitespace changes",
            Action::ExpandContext => "Show more context",
            Action::FullFile => "Show the whole file",
            Action::ToggleViewed => "Toggle file viewed (syncs)",
            Action::NextUnviewed => "Next unviewed file",
            Action::MarkReviewed => "Toggle change reviewed",
            Action::Search => "Search (on a list: filter it)",
            Action::SearchNext => "Next search match",
            Action::SearchPrev => "Previous search match",
            Action::FindFile => "Go to file",
            Action::Comment => "Comment",
            Action::VisualLines => "Select lines (for multi-line comments)",
            Action::Suggest => "Suggest a change (opens $EDITOR)",
            Action::ReplyThread => "Reply to the thread",
            Action::ResolveThread => "Resolve or unresolve the thread",
            Action::DeleteDraft => "Delete the draft comment",
            Action::FileComment => "Comment on the whole file",
            Action::SubmitReview => "Submit your review",
            Action::NextThread => "Next unresolved thread",
            Action::PrevThread => "Previous unresolved thread",
            Action::ToggleSinceReview => "Show only changes since your last review",
            Action::JumpMove => "Jump to the other end of moved code",
            Action::PickCommits => "Choose commits to view",
            Action::NextTab => "Next tab",
            Action::PrevTab => "Previous tab",
            Action::GoHome => "Go home",
            Action::GoCode => "Go to the repository's code",
            Action::GoIssues => "Go to the repository's issues",
            Action::GoPulls => "Go to the repository's pull requests",
            Action::Tab1 => "First tab",
            Action::Tab2 => "Second tab",
            Action::Tab3 => "Third tab",
            Action::Tab4 => "Fourth tab",
            Action::Star => "Star / unstar",
            Action::Hints => "Follow a link by its letters",
            Action::HintsBrowser => "Follow a link, in the browser",
            Action::Forward => "Forward",
            Action::Copy => "Copy the link",
            Action::Menu => "Everything you can do here",
            Action::Branch => "Switch branches or tags",
            Action::ToggleState => "Open / closed / all",
            Action::Sort => "Change the sort",
            Action::ScrollDown => "Scroll down a line",
            Action::ScrollUp => "Scroll up a line",
            Action::PageDown => "Page down",
            Action::PageUp => "Page up",
            Action::UpLevel => "Up a level",
        }
    }

    fn defaults(self) -> &'static [&'static str] {
        match self {
            Action::Down => &["<Down>", "j"],
            Action::Up => &["<Up>", "k"],
            Action::HalfPageDown => &["<C-d>"],
            Action::HalfPageUp => &["<C-u>"],
            Action::Top => &["<Home>", "g"],
            Action::Bottom => &["<End>", "G"],
            Action::Open => &["<Enter>"],
            Action::Back => &["<Esc>", "<BS>"],
            Action::Close => &[],
            Action::Quit => &["q", "<C-c>"],
            Action::Refresh => &["r"],
            Action::OpenInBrowser => &["o"],
            Action::CommandPalette => &[":", "<C-k>"],
            Action::Help => &["?"],
            Action::NextHunk => &["n"],
            Action::PrevHunk => &["p"],
            Action::NextFile => &["<S-Down>"],
            Action::PrevFile => &["<S-Up>"],
            Action::ToggleTree => &[],
            Action::SwitchPane => &["<Tab>"],
            Action::ToggleSplit => &[],
            Action::IgnoreWhitespace => &["w"],
            Action::ExpandContext => &["e"],
            Action::FullFile => &[],
            Action::ToggleViewed => &["v"],
            Action::NextUnviewed => &[],
            Action::MarkReviewed => &["m"],
            Action::Search => &["/"],
            Action::SearchNext => &[],
            Action::SearchPrev => &[],
            Action::FindFile => &["f"],
            Action::Comment => &["c"],
            Action::VisualLines => &["x"],
            Action::Suggest => &[],
            Action::ReplyThread => &[],
            Action::ResolveThread => &[],
            Action::DeleteDraft => &["<Delete>"],
            Action::FileComment => &[],
            Action::SubmitReview => &["a"],
            Action::NextThread => &[],
            Action::PrevThread => &[],
            Action::ToggleSinceReview => &[],
            Action::JumpMove => &[],
            Action::PickCommits => &[],
            Action::NextTab => &["<Right>"],
            Action::PrevTab => &["<Left>"],
            Action::GoHome => &["h"],
            Action::GoCode => &[],
            Action::GoIssues => &[],
            Action::GoPulls => &[],
            Action::Tab1 => &["1"],
            Action::Tab2 => &["2"],
            Action::Tab3 => &["3"],
            Action::Tab4 => &["4"],
            Action::Star => &["s"],
            Action::Hints => &["l"],
            Action::HintsBrowser => &[],
            Action::Forward => &[],
            Action::Copy => &["y"],
            Action::Menu => &["<Space>"],
            Action::Branch => &["b"],
            Action::ToggleState => &[],
            Action::Sort => &[],
            Action::ScrollDown => &[],
            Action::ScrollUp => &[],
            Action::PageDown => &["<PageDown>"],
            Action::PageUp => &["<PageUp>"],
            Action::UpLevel => &["u"],
        }
    }

    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.into_iter().find(|a| a.name() == name)
    }
}

/// A normalized key press. Shift is folded into the character for printable
/// keys, so `G` is `Char('G')` with no modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub code: KeyCode,
    pub mods: KeyModifiers,
}

impl Key {
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        let mods = match code {
            KeyCode::Char(_) => mods - KeyModifiers::SHIFT,
            _ => mods,
        };
        Self { code, mods }
    }
}

impl From<KeyEvent> for Key {
    fn from(event: KeyEvent) -> Self {
        Key::new(event.code, event.modifiers)
    }
}

const NAMED: &[(&str, KeyCode)] = &[
    ("Enter", KeyCode::Enter),
    ("CR", KeyCode::Enter),
    ("Esc", KeyCode::Esc),
    ("Tab", KeyCode::Tab),
    ("S-Tab", KeyCode::BackTab),
    ("BS", KeyCode::Backspace),
    ("Space", KeyCode::Char(' ')),
    ("Up", KeyCode::Up),
    ("Down", KeyCode::Down),
    ("Left", KeyCode::Left),
    ("Right", KeyCode::Right),
    ("PageUp", KeyCode::PageUp),
    ("PageDown", KeyCode::PageDown),
    ("Home", KeyCode::Home),
    ("End", KeyCode::End),
    ("Delete", KeyCode::Delete),
    ("Del", KeyCode::Delete),
    ("lt", KeyCode::Char('<')),
];

/// Parses a sequence in vim notation.
pub fn parse_sequence(s: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '<' || chars.peek().is_none() {
            keys.push(Key::new(KeyCode::Char(c), KeyModifiers::NONE));
            continue;
        }
        let mut name = String::new();
        loop {
            match chars.next() {
                Some('>') => break,
                Some(c) => name.push(c),
                None => return Err(format!("unclosed `<` in key sequence `{s}`")),
            }
        }
        keys.push(parse_named(&name).ok_or_else(|| format!("unknown key `<{name}>` in `{s}`"))?);
    }
    if keys.is_empty() {
        return Err("empty key sequence".into());
    }
    Ok(keys)
}

fn parse_named(name: &str) -> Option<Key> {
    if let Some((_, code)) = NAMED.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
        return Some(Key::new(*code, KeyModifiers::NONE));
    }
    let mut mods = KeyModifiers::NONE;
    let mut rest = name;
    loop {
        let (modifier, tail) = match rest.split_once('-') {
            Some((m, tail)) if !tail.is_empty() => (m, tail),
            _ => break,
        };
        mods |= match modifier.to_ascii_uppercase().as_str() {
            "C" => KeyModifiers::CONTROL,
            "A" | "M" => KeyModifiers::ALT,
            "S" => KeyModifiers::SHIFT,
            _ => return None,
        };
        rest = tail;
    }
    if mods.is_empty() {
        return None;
    }
    let mut chars = rest.chars();
    let code = match (chars.next(), chars.next()) {
        (Some(c), None) => KeyCode::Char(c.to_ascii_lowercase()),
        _ => NAMED
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(rest))
            .map(|(_, code)| *code)?,
    };
    Some(Key::new(code, mods))
}

pub fn format_sequence(keys: &[Key]) -> String {
    keys.iter().map(|k| format_key(*k)).collect()
}

fn format_key(key: Key) -> String {
    let base = match key.code {
        KeyCode::Char('<') => "lt".to_owned(),
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(c) if key.mods.is_empty() => return c.to_string(),
        KeyCode::Char(c) => c.to_string(),
        code => NAMED
            .iter()
            .find(|(_, c)| *c == code)
            .map_or_else(|| format!("{code:?}"), |(n, _)| (*n).to_owned()),
    };
    let mut prefix = String::new();
    if key.mods.contains(KeyModifiers::CONTROL) {
        prefix.push_str("C-");
    }
    if key.mods.contains(KeyModifiers::ALT) {
        prefix.push_str("A-");
    }
    if key.mods.contains(KeyModifiers::SHIFT) {
        prefix.push_str("S-");
    }
    format!("<{prefix}{base}>")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    Action(Action),
    /// A prefix of at least one binding; wait for more keys.
    Pending,
    Unbound,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    /// `(keys, action, where)`.
    bindings: Vec<(Vec<Key>, Action, Scope)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::with_overrides(&HashMap::new()).expect("default keymap is valid")
    }
}

/// `page:h` binds `h` on pages only, `diff:s` in the diff only; a plain
/// sequence applies where its action does.
fn scoped(seq: &str, action: Action) -> (&str, Scope) {
    if let Some(rest) = seq.strip_prefix("page:") {
        (rest, Scope::Page)
    } else if let Some(rest) = seq.strip_prefix("diff:") {
        (rest, Scope::Diff)
    } else {
        (seq, action.scope())
    }
}

impl Keymap {
    /// The defaults, with each action named in `overrides` rebound to exactly
    /// the given sequences (an empty list unbinds it).
    pub fn with_overrides(overrides: &HashMap<String, Vec<String>>) -> Result<Self, String> {
        for name in overrides.keys() {
            if Action::from_name(name).is_none() {
                let known: Vec<_> = Action::ALL.iter().map(|a| a.name()).collect();
                return Err(format!(
                    "unknown action `{name}` in [keys]; known actions: {}",
                    known.join(", ")
                ));
            }
        }
        let mut bindings = Vec::new();
        for action in Action::ALL {
            let sequences: Vec<String> = match overrides.get(action.name()) {
                Some(list) => list.clone(),
                None => action.defaults().iter().map(|s| (*s).to_owned()).collect(),
            };
            for seq in sequences {
                let (seq, scope) = scoped(&seq, action);
                let keys =
                    parse_sequence(seq).map_err(|e| format!("[keys] {}: {e}", action.name()))?;
                bindings.push((keys, action, scope));
            }
        }
        let keymap = Self { bindings };
        keymap.validate()?;
        Ok(keymap)
    }

    /// Rejects duplicate bindings and bindings that are a prefix of another
    /// (the shorter one would make the longer unreachable).
    fn validate(&self) -> Result<(), String> {
        for (i, (a, action_a, scope_a)) in self.bindings.iter().enumerate() {
            for (b, action_b, scope_b) in &self.bindings[i + 1..] {
                let shorter = a.len().min(b.len());
                if a[..shorter] == b[..shorter] && scope_a.overlaps(*scope_b) {
                    return Err(format!(
                        "key `{}` ({}) conflicts with `{}` ({})",
                        format_sequence(a),
                        action_a.name(),
                        format_sequence(b),
                        action_b.name()
                    ));
                }
            }
        }
        Ok(())
    }

    /// The action `keys` trigger in `scope`.
    pub fn resolve(&self, keys: &[Key], scope: Scope) -> Resolution {
        let mut pending = false;
        for (binding, action, where_) in &self.bindings {
            if !where_.overlaps(scope) {
                continue;
            }
            if binding.as_slice() == keys {
                return Resolution::Action(*action);
            }
            if binding.len() > keys.len() && binding.starts_with(keys) {
                pending = true;
            }
        }
        if pending {
            Resolution::Pending
        } else {
            Resolution::Unbound
        }
    }

    /// Bindings that start with `keys` in `scope`: the keys still to type,
    /// and the action.
    pub fn continuations(&self, keys: &[Key], scope: Scope) -> Vec<(Vec<Key>, Action)> {
        self.bindings
            .iter()
            .filter(|(binding, _, where_)| {
                where_.overlaps(scope) && binding.len() > keys.len() && binding.starts_with(keys)
            })
            .map(|(binding, action, _)| (binding[keys.len()..].to_vec(), *action))
            .collect()
    }

    /// Every sequence bound to `action`, anywhere.
    pub fn keys_for(&self, action: Action) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|(_, a, _)| *a == action)
            .map(|(keys, _, _)| format_sequence(keys))
            .collect()
    }

    /// The sequences that run `action` in `scope`, for hints.
    pub fn keys_in(&self, action: Action, scope: Scope) -> Vec<Vec<Key>> {
        self.bindings
            .iter()
            .filter(|(_, a, where_)| *a == action && where_.overlaps(scope))
            .map(|(keys, _, _)| keys.clone())
            .collect()
    }
}

/// A key as people write it: `↵`, `esc`, `⇥`, `space`, `ctrl-d`.
pub fn pretty(keys: &[Key]) -> String {
    keys.iter()
        .map(|k| {
            let base = match k.code {
                KeyCode::Enter => "↵".to_owned(),
                KeyCode::Esc => "esc".to_owned(),
                KeyCode::Tab => "⇥".to_owned(),
                KeyCode::BackTab => "⇧⇥".to_owned(),
                KeyCode::Backspace => "⌫".to_owned(),
                KeyCode::Char(' ') => "space".to_owned(),
                KeyCode::Up => "↑".to_owned(),
                KeyCode::Down => "↓".to_owned(),
                KeyCode::Left => "←".to_owned(),
                KeyCode::Right => "→".to_owned(),
                KeyCode::PageUp => "pgup".to_owned(),
                KeyCode::PageDown => "pgdn".to_owned(),
                KeyCode::Home => "home".to_owned(),
                KeyCode::End => "end".to_owned(),
                KeyCode::Delete => "del".to_owned(),
                KeyCode::F(n) => format!("f{n}"),
                KeyCode::Char(c) => c.to_string(),
                code => format!("{code:?}").to_lowercase(),
            };
            if k.mods.contains(KeyModifiers::CONTROL) {
                format!("ctrl-{base}")
            } else if k.mods.contains(KeyModifiers::ALT) {
                format!("alt-{base}")
            } else if k.mods.contains(KeyModifiers::SHIFT) {
                format!("⇧{base}")
            } else {
                base
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> Key {
        Key::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn parses_vim_notation() {
        assert_eq!(parse_sequence("gg").unwrap(), vec![key('g'), key('g')]);
        assert_eq!(parse_sequence("]h").unwrap(), vec![key(']'), key('h')]);
        assert_eq!(
            parse_sequence("<C-d>").unwrap(),
            vec![Key::new(KeyCode::Char('d'), KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_sequence("<c-D>").unwrap(),
            vec![Key::new(KeyCode::Char('d'), KeyModifiers::CONTROL)]
        );
        assert_eq!(
            parse_sequence("<Enter>").unwrap(),
            vec![Key::new(KeyCode::Enter, KeyModifiers::NONE)]
        );
        assert_eq!(parse_sequence("<lt>").unwrap(), vec![key('<')]);
        assert_eq!(parse_sequence("<").unwrap(), vec![key('<')]);
        assert!(parse_sequence("<Nope>").is_err());
        assert!(parse_sequence("<C-d").is_err());
        assert!(parse_sequence("").is_err());
    }

    #[test]
    fn formats_round_trip() {
        for s in [
            "gg", "G", "]h", "<C-d>", "<Enter>", "<Esc>", "<S-Tab>", "<lt>", "?", ":",
        ] {
            assert_eq!(format_sequence(&parse_sequence(s).unwrap()), s);
        }
    }

    #[test]
    fn shift_is_folded_into_chars() {
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(Key::from(event), key('G'));
    }

    #[test]
    fn every_default_is_one_key() {
        let keymap = Keymap::default();
        for (keys, action, _) in &keymap.bindings {
            assert_eq!(
                keys.len(),
                1,
                "{} is bound to {}",
                action.name(),
                format_sequence(keys)
            );
        }
        assert_eq!(
            keymap.resolve(&[key('j')], Scope::Page),
            Resolution::Action(Action::Down)
        );
        assert_eq!(
            keymap.resolve(&[key('g')], Scope::Page),
            Resolution::Action(Action::Top)
        );
        assert_eq!(
            keymap.resolve(&[key('z')], Scope::Page),
            Resolution::Unbound
        );
    }

    #[test]
    fn sequences_still_work_when_configured() {
        let overrides = HashMap::from([("go_issues".to_owned(), vec!["zi".to_owned()])]);
        let keymap = Keymap::with_overrides(&overrides).unwrap();
        assert_eq!(
            keymap.resolve(&[key('z')], Scope::Page),
            Resolution::Pending
        );
        assert_eq!(
            keymap.resolve(&[key('z'), key('i')], Scope::Page),
            Resolution::Action(Action::GoIssues)
        );
    }

    #[test]
    fn every_key_means_one_thing_everywhere() {
        let keymap = Keymap::default();
        for (i, (a, action_a, _)) in keymap.bindings.iter().enumerate() {
            for (b, action_b, _) in &keymap.bindings[i + 1..] {
                assert!(
                    a != b,
                    "{} is both {} and {}",
                    format_sequence(a),
                    action_a.name(),
                    action_b.name()
                );
            }
        }
        // The verbs do the same on a page and in the diff.
        for c in ['f', 'c', 'o', 'y', '/', 'u'] {
            assert_eq!(
                keymap.resolve(&[key(c)], Scope::Page),
                keymap.resolve(&[key(c)], Scope::Diff),
                "{c}"
            );
        }
        // A page key can't shadow a global one.
        let clash = HashMap::from([("star".to_owned(), vec!["q".to_owned()])]);
        assert!(
            Keymap::with_overrides(&clash)
                .unwrap_err()
                .contains("conflicts")
        );
        // Config can scope a binding.
        let scoped = HashMap::from([("star".to_owned(), vec!["page:z".to_owned()])]);
        assert!(Keymap::with_overrides(&scoped).is_ok());
    }

    #[test]
    fn keys_read_like_people_write_them() {
        assert_eq!(pretty(&parse_sequence("<Enter>").unwrap()), "↵");
        assert_eq!(pretty(&parse_sequence("<C-d>").unwrap()), "ctrl-d");
        assert_eq!(pretty(&parse_sequence("<Space>").unwrap()), "space");
        assert_eq!(pretty(&parse_sequence("G").unwrap()), "G");
    }

    #[test]
    fn overrides_replace_defaults() {
        let overrides = HashMap::from([("down".to_owned(), vec!["z".to_owned()])]);
        let keymap = Keymap::with_overrides(&overrides).unwrap();
        assert_eq!(
            keymap.resolve(&[key('z')], Scope::Page),
            Resolution::Action(Action::Down)
        );
        assert_eq!(
            keymap.resolve(&[key('j')], Scope::Page),
            Resolution::Unbound
        );
        assert_eq!(keymap.keys_for(Action::Down), ["z"]);
    }

    #[test]
    fn rejects_bad_overrides() {
        let unknown = HashMap::from([("fly".to_owned(), vec!["f".to_owned()])]);
        assert!(
            Keymap::with_overrides(&unknown)
                .unwrap_err()
                .contains("unknown action")
        );
        let conflict = HashMap::from([("down".to_owned(), vec!["k".to_owned()])]);
        assert!(
            Keymap::with_overrides(&conflict)
                .unwrap_err()
                .contains("conflicts")
        );
        let shadow = HashMap::from([("refresh".to_owned(), vec!["gx".to_owned()])]);
        assert!(
            Keymap::with_overrides(&shadow)
                .unwrap_err()
                .contains("conflicts")
        );
    }
}
