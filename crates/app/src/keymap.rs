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
    pub const ALL: [Action; 66] = [
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
            | Action::Copy => Scope::Global,
            Action::NextTab
            | Action::PrevTab
            | Action::GoCode
            | Action::Star
            | Action::Hints
            | Action::HintsBrowser
            | Action::Menu
            | Action::Branch
            | Action::ToggleState
            | Action::Sort
            | Action::ScrollDown
            | Action::ScrollUp
            | Action::PageDown => Scope::Page,
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
            Action::Search => "Search GitHub, or filter the list (in a diff: search it)",
            Action::SearchNext => "Next search match",
            Action::SearchPrev => "Previous search match",
            Action::FindFile => "Go to file",
            Action::Comment => "Comment (on the issue, PR, or diff line)",
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
            Action::Star => "Star or unstar the repository",
            Action::Hints => "Follow a link by its letters",
            Action::HintsBrowser => "Open a link in the browser by its letters",
            Action::Forward => "Forward (after going back)",
            Action::Copy => "Copy the link (selected item, or the page)",
            Action::Menu => "Everything you can do here",
            Action::Branch => "Switch branches or tags",
            Action::ToggleState => "Show open, closed or all",
            Action::Sort => "Change the sort",
            Action::ScrollDown => "Scroll down a line",
            Action::ScrollUp => "Scroll up a line",
            Action::PageDown => "Page down",
        }
    }

    fn defaults(self) -> &'static [&'static str] {
        match self {
            Action::Down => &["j", "<Down>"],
            Action::Up => &["k", "<Up>"],
            Action::HalfPageDown => &["<C-d>"],
            Action::HalfPageUp => &["<C-u>"],
            Action::Top => &["gg"],
            Action::Bottom => &["G"],
            Action::Open => &["<Enter>"],
            Action::Back => &["<Esc>", "<BS>", "H"],
            Action::Close => &[],
            Action::Quit => &["q", "<C-c>"],
            Action::Refresh => &["r"],
            Action::OpenInBrowser => &["o"],
            Action::CommandPalette => &[":", "<C-k>"],
            Action::Help => &["?"],
            Action::NextHunk => &["]h"],
            Action::PrevHunk => &["[h"],
            Action::NextFile => &["]f"],
            Action::PrevFile => &["[f"],
            Action::ToggleTree => &["<Tab>"],
            Action::SwitchPane => &["<C-w>"],
            Action::ToggleSplit => &["s"],
            Action::IgnoreWhitespace => &["w"],
            Action::ExpandContext => &["x"],
            Action::FullFile => &["F"],
            Action::ToggleViewed => &["v"],
            Action::NextUnviewed => &["]u"],
            Action::MarkReviewed => &["m"],
            Action::Search => &["/"],
            Action::SearchNext => &["n"],
            Action::SearchPrev => &["N"],
            Action::FindFile => &["t", "gf"],
            Action::Comment => &["c"],
            Action::VisualLines => &["V"],
            Action::Suggest => &["S"],
            Action::ReplyThread => &["a"],
            Action::ResolveThread => &["R"],
            Action::DeleteDraft => &["D"],
            Action::FileComment => &["f"],
            Action::SubmitReview => &["gr"],
            Action::NextThread => &["]c"],
            Action::PrevThread => &["[c"],
            Action::ToggleSinceReview => &["gl"],
            Action::JumpMove => &["gm"],
            Action::PickCommits => &["gc"],
            Action::NextTab => &["<Tab>"],
            Action::PrevTab => &["<S-Tab>"],
            Action::GoHome => &["gh"],
            Action::GoCode => &["gc"],
            Action::GoIssues => &["gi"],
            Action::GoPulls => &["gp"],
            Action::Tab1 => &["1"],
            Action::Tab2 => &["2"],
            Action::Tab3 => &["3"],
            Action::Tab4 => &["4"],
            Action::Star => &["*"],
            Action::Hints => &["f"],
            Action::HintsBrowser => &["F"],
            Action::Forward => &["L"],
            Action::Copy => &["y"],
            Action::Menu => &["."],
            Action::Branch => &["w"],
            Action::ToggleState => &["S"],
            Action::Sort => &["O"],
            Action::ScrollDown => &["J"],
            Action::ScrollUp => &["K"],
            Action::PageDown => &["<Space>", "<PageDown>"],
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
    bindings: Vec<(Vec<Key>, Action)>,
}

impl Default for Keymap {
    fn default() -> Self {
        Self::with_overrides(&HashMap::new()).expect("default keymap is valid")
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
                let keys =
                    parse_sequence(&seq).map_err(|e| format!("[keys] {}: {e}", action.name()))?;
                bindings.push((keys, action));
            }
        }
        let keymap = Self { bindings };
        keymap.validate()?;
        Ok(keymap)
    }

    /// Rejects duplicate bindings and bindings that are a prefix of another
    /// (the shorter one would make the longer unreachable).
    fn validate(&self) -> Result<(), String> {
        for (i, (a, action_a)) in self.bindings.iter().enumerate() {
            for (b, action_b) in &self.bindings[i + 1..] {
                let shorter = a.len().min(b.len());
                if a[..shorter] == b[..shorter] && action_a.scope().overlaps(action_b.scope()) {
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
        for (binding, action) in &self.bindings {
            if !action.scope().overlaps(scope) {
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
            .filter(|(binding, action)| {
                action.scope().overlaps(scope)
                    && binding.len() > keys.len()
                    && binding.starts_with(keys)
            })
            .map(|(binding, action)| (binding[keys.len()..].to_vec(), *action))
            .collect()
    }

    pub fn keys_for(&self, action: Action) -> Vec<String> {
        self.bindings
            .iter()
            .filter(|(_, a)| *a == action)
            .map(|(keys, _)| format_sequence(keys))
            .collect()
    }
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
    fn resolves_sequences() {
        let keymap = Keymap::default();
        assert_eq!(
            keymap.resolve(&[key('j')], Scope::Page),
            Resolution::Action(Action::Down)
        );
        assert_eq!(
            keymap.resolve(&[key('g')], Scope::Page),
            Resolution::Pending
        );
        assert_eq!(
            keymap.resolve(&[key('g'), key('g')], Scope::Page),
            Resolution::Action(Action::Top)
        );
        assert_eq!(
            keymap.resolve(&[key('z')], Scope::Page),
            Resolution::Unbound
        );
    }

    #[test]
    fn scopes_share_keys() {
        let keymap = Keymap::default();
        assert_eq!(
            keymap.resolve(&[key('f')], Scope::Page),
            Resolution::Action(Action::Hints)
        );
        assert_eq!(
            keymap.resolve(&[key('f')], Scope::Diff),
            Resolution::Action(Action::FileComment)
        );
        // A page key can't shadow a global one.
        let clash = HashMap::from([("star".to_owned(), vec!["q".to_owned()])]);
        assert!(
            Keymap::with_overrides(&clash)
                .unwrap_err()
                .contains("conflicts")
        );
    }

    #[test]
    fn overrides_replace_defaults() {
        let overrides = HashMap::from([("down".to_owned(), vec!["e".to_owned()])]);
        let keymap = Keymap::with_overrides(&overrides).unwrap();
        assert_eq!(
            keymap.resolve(&[key('e')], Scope::Page),
            Resolution::Action(Action::Down)
        );
        assert_eq!(
            keymap.resolve(&[key('j')], Scope::Page),
            Resolution::Unbound
        );
        assert_eq!(keymap.keys_for(Action::Down), ["e"]);
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
        let shadow = HashMap::from([("refresh".to_owned(), vec!["g".to_owned()])]);
        assert!(
            Keymap::with_overrides(&shadow)
                .unwrap_err()
                .contains("conflicts")
        );
    }
}
