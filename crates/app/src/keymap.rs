//! Rebindable keymap. A binding is one key, in vim notation: `j`, `G`,
//! `<C-d>`, `<Enter>`, `<Esc>`, `<Tab>`, `<Down>`.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Where a binding applies. Diff and page bindings may share keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Global,
    Diff,
    Page,
}

impl Scope {
    pub fn overlaps(self, other: Scope) -> bool {
        self == Scope::Global || other == Scope::Global || self == other
    }
}

/// Declares [`Action`] from one table: config name, default keys, scope
/// and description.
macro_rules! actions {
    ($($action:ident $name:literal [$($key:literal),*] $scope:ident $description:literal;)*) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum Action {
            $($action,)*
        }

        impl Action {
            pub const ALL: &[Action] = &[$(Action::$action),*];

            /// The name used in the `[keys]` config table.
            pub fn name(self) -> &'static str {
                match self {
                    $(Action::$action => $name,)*
                }
            }

            pub fn description(self) -> &'static str {
                match self {
                    $(Action::$action => $description,)*
                }
            }

            pub fn scope(self) -> Scope {
                match self {
                    $(Action::$action => Scope::$scope,)*
                }
            }

            fn defaults(self) -> &'static [&'static str] {
                match self {
                    $(Action::$action => &[$($key),*],)*
                }
            }
        }
    };
}

actions! {
    Down              "down"              ["<Down>", "j"]      Global "Move down";
    Up                "up"                ["<Up>", "k"]        Global "Move up";
    HalfPageDown      "half_page_down"    ["<C-d>"]            Global "Half page down";
    HalfPageUp        "half_page_up"      ["<C-u>"]            Global "Half page up";
    PageDown          "page_down"         ["<PageDown>"]       Global "Page down";
    PageUp            "page_up"           ["<PageUp>"]         Global "Page up";
    Top               "top"               ["<Home>", "g"]      Global "Go to top";
    Bottom            "bottom"            ["<End>", "G"]       Global "Go to bottom";
    Open              "open"              ["<Enter>"]          Global "Open the selection";
    Back              "back"              ["<Esc>", "<BS>", "<A-Left>"] Global "Back";
    Forward           "forward"           ["<A-Right>"]        Global "Forward";
    UpLevel           "up_level"          ["u"]                Global "Up a level";
    NextTab           "next_tab"          ["<Right>", "l"]     Global "Next tab";
    PrevTab           "prev_tab"          ["<Left>", "h"]      Global "Previous tab";
    Tab1              "tab_1"             ["1"]                Global "First tab";
    Tab2              "tab_2"             ["2"]                Global "Second tab";
    Tab3              "tab_3"             ["3"]                Global "Third tab";
    Tab4              "tab_4"             ["4"]                Global "Fourth tab";
    GoHome            "go_home"           ["H"]                Global "Go home";
    Search            "search"            ["/"]                Global "Search (on a list: filter it)";
    FindFile          "find_file"         ["f"]                Global "Go to file";
    Comment           "comment"           ["c"]                Global "Comment (on a thread: reply)";
    Copy              "copy_link"         ["y"]                Global "Copy the link";
    OpenInBrowser     "open_in_browser"   ["o"]                Global "Open on GitHub";
    Refresh           "refresh"           ["r"]                Global "Refresh";
    Menu              "actions_menu"      ["<Space>", "?"]     Global "Everything you can do here";
    CommandPalette    "command_palette"   [":", "<C-k>"]       Global "Command palette";
    Messages          "messages"          []                   Global "Recent messages and errors";
    Quit              "quit"              ["q", "<C-c>"]       Global "Quit";

    Star              "star"              ["s"]                Page   "Star / unstar";
    Branch            "branch"            ["b"]                Page   "Switch branches or tags";
    Hints             "hints"             ["i"]                Page   "Follow a link by its letters";
    HintsBrowser      "hints_browser"     ["I"]                Page   "Follow a link, in the browser";
    ToggleState       "toggle_state"      []                   Page   "Open / closed / all";
    Sort              "sort"              []                   Page   "Change the sort";

    NextHunk          "next_hunk"         ["n"]                Diff   "Next change";
    PrevHunk          "prev_hunk"         ["p"]                Diff   "Previous change";
    NextFile          "next_file"         ["<S-Down>"]         Diff   "Next file";
    PrevFile          "prev_file"         ["<S-Up>"]           Diff   "Previous file";
    NextUnviewed      "next_unviewed"     ["U"]                Diff   "Next unviewed file";
    NextThread        "next_thread"       ["N"]                Diff   "Next unresolved thread";
    PrevThread        "prev_thread"       ["P"]                Diff   "Previous unresolved thread";
    JumpMove          "jump_move"         ["M"]                Diff   "Jump to the other end of moved code";
    SearchNext        "search_next"       []                   Diff   "Next search match";
    SearchPrev        "search_prev"       []                   Diff   "Previous search match";
    SwitchPane        "switch_pane"       ["<Tab>"]            Diff   "Switch between the file tree and the diff";
    ToggleTree        "toggle_tree"       ["t"]                Diff   "Show or hide the file tree";
    ToggleSplit       "toggle_split"      ["S"]                Diff   "Split or unified view";
    IgnoreWhitespace  "ignore_whitespace" ["w"]                Diff   "Ignore whitespace changes";
    ExpandContext     "expand_context"    ["e"]                Diff   "Show more context";
    FullFile          "full_file"         ["F"]                Diff   "Show the whole file";
    ToggleSinceReview "since_review"      []                   Diff   "Show only changes since your last review";
    PickCommits       "pick_commits"      []                   Diff   "Choose commits to view";
    ToggleViewed      "toggle_viewed"     ["v"]                Diff   "Mark the file viewed (syncs with GitHub)";
    MarkReviewed      "mark_reviewed"     ["m"]                Diff   "Mark the change reviewed";
    VisualLines       "visual_lines"      ["x"]                Diff   "Select lines (for multi-line comments)";
    Suggest           "suggest"           []                   Diff   "Suggest a change (opens $EDITOR)";
    FileComment       "file_comment"      ["C"]                Diff   "Comment on the whole file";
    ResolveThread     "resolve"           ["R"]                Diff   "Resolve or unresolve the thread";
    DeleteDraft       "delete_draft"      ["<Delete>"]         Diff   "Delete the draft comment";
    UndoDelete        "undo_delete"       ["<C-z>"]            Diff   "Bring back the draft just deleted";
    SubmitReview      "submit_review"     ["a"]                Diff   "Submit your review";
}

impl Action {
    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }

    /// Says where the action works, for when it's used elsewhere.
    pub fn not_here(self) -> String {
        let place = match self.scope() {
            Scope::Diff => "in a pull request's Files changed tab",
            Scope::Page => "on pages, not in the diff",
            Scope::Global => "not here",
        };
        format!("{}: {place}", self.description())
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
    /// Folds Shift into the key where the key already says it: `G`, and
    /// Shift-Tab (terminals send `BackTab` with Shift; `<C-S-Tab>` reads as
    /// Tab with Shift).
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        let code = match code {
            KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            code => code,
        };
        let mods = match code {
            KeyCode::Char(_) | KeyCode::BackTab => mods - KeyModifiers::SHIFT,
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

/// Parses one key in vim notation.
pub fn parse_key(s: &str) -> Result<Key, String> {
    let mut chars = s.chars();
    let (Some(first), rest) = (chars.next(), chars.as_str()) else {
        return Err("empty key".into());
    };
    if rest.is_empty() {
        return Ok(Key::new(KeyCode::Char(first), KeyModifiers::NONE));
    }
    match s.strip_prefix('<').and_then(|n| n.strip_suffix('>')) {
        Some(name) if !name.contains(['<', '>']) => {
            parse_named(name).ok_or_else(|| format!("unknown key `{s}`"))
        }
        _ if s.starts_with('<') && !s.contains('>') => Err(format!("unclosed `<` in `{s}`")),
        _ => Err(format!(
            "`{s}` is more than one key; bindings are single keys"
        )),
    }
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

/// Writes `key` in vim notation.
pub fn format_key(key: Key) -> String {
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

#[derive(Debug, Clone)]
pub struct Keymap {
    /// `(key, action, where)`.
    bindings: Vec<(Key, Action, Scope)>,
}

impl Default for Keymap {
    /// The default bindings (a test checks they're valid).
    fn default() -> Self {
        Self::with_overrides(&HashMap::new()).unwrap_or(Self {
            bindings: Vec::new(),
        })
    }
}

/// `page:h` binds `h` on pages only, `diff:s` in the diff only; a plain
/// key applies where its action does.
fn scoped(key: &str, action: Action) -> (&str, Scope) {
    if let Some(rest) = key.strip_prefix("page:") {
        (rest, Scope::Page)
    } else if let Some(rest) = key.strip_prefix("diff:") {
        (rest, Scope::Diff)
    } else {
        (key, action.scope())
    }
}

impl Keymap {
    /// The defaults, with each action named in `overrides` rebound to exactly
    /// the given keys (an empty list unbinds it).
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
        for &action in Action::ALL {
            let keys: Vec<String> = match overrides.get(action.name()) {
                Some(list) => list.clone(),
                None => action.defaults().iter().map(|s| (*s).to_owned()).collect(),
            };
            for key in keys {
                let (key, scope) = scoped(&key, action);
                let key = parse_key(key).map_err(|e| format!("[keys] {}: {e}", action.name()))?;
                bindings.push((key, action, scope));
            }
        }
        let keymap = Self { bindings };
        keymap.validate()?;
        Ok(keymap)
    }

    /// Rejects a key bound twice where both bindings apply.
    fn validate(&self) -> Result<(), String> {
        for (i, (a, action_a, scope_a)) in self.bindings.iter().enumerate() {
            for (b, action_b, scope_b) in self.bindings.iter().skip(i + 1) {
                if a == b && scope_a.overlaps(*scope_b) {
                    return Err(format!(
                        "key `{}` ({}) conflicts with {}",
                        format_key(*a),
                        action_a.name(),
                        action_b.name()
                    ));
                }
            }
        }
        Ok(())
    }

    /// The action `key` runs in `scope`.
    pub fn resolve(&self, key: Key, scope: Scope) -> Option<Action> {
        self.bindings
            .iter()
            .find(|(k, _, where_)| *k == key && where_.overlaps(scope))
            .map(|(_, action, _)| *action)
    }

    /// The keys that run `action` in `scope`, for hints.
    pub fn keys_in(&self, action: Action, scope: Scope) -> impl Iterator<Item = Key> {
        self.bindings
            .iter()
            .filter(move |(_, a, where_)| *a == action && where_.overlaps(scope))
            .map(|(key, _, _)| *key)
    }
}

/// A key as people write it: `↵`, `esc`, `⇥`, `space`, `ctrl-d`.
pub fn pretty(key: Key) -> String {
    let base = match key.code {
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
    if key.mods.contains(KeyModifiers::CONTROL) {
        format!("ctrl-{base}")
    } else if key.mods.contains(KeyModifiers::ALT) {
        format!("alt-{base}")
    } else if key.mods.contains(KeyModifiers::SHIFT) {
        format!("⇧{base}")
    } else {
        base
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
        assert_eq!(parse_key("G").unwrap(), key('G'));
        assert_eq!(
            parse_key("<C-d>").unwrap(),
            Key::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            parse_key("<c-D>").unwrap(),
            Key::new(KeyCode::Char('d'), KeyModifiers::CONTROL)
        );
        assert_eq!(
            parse_key("<Enter>").unwrap(),
            Key::new(KeyCode::Enter, KeyModifiers::NONE)
        );
        assert_eq!(parse_key("<lt>").unwrap(), key('<'));
        assert_eq!(parse_key("<").unwrap(), key('<'));
        assert!(parse_key("<Nope>").is_err());
        assert!(parse_key("<C-d").is_err());
        assert!(parse_key("").is_err());
    }

    /// Bindings are single keys: a sequence is an error that says so.
    #[test]
    fn sequences_are_rejected() {
        for s in ["gg", "]h", "<C-d><C-u>", "<Esc>j"] {
            let err = parse_key(s).unwrap_err();
            assert!(err.contains("single keys"), "{s}: {err}");
        }
        let overrides = HashMap::from([("sort".to_owned(), vec!["zs".to_owned()])]);
        let err = Keymap::with_overrides(&overrides).unwrap_err();
        assert!(err.contains("single keys"), "{err}");
    }

    #[test]
    fn formats_round_trip() {
        for s in [
            "G", "<C-d>", "<Enter>", "<Esc>", "<S-Tab>", "<lt>", "?", ":",
        ] {
            assert_eq!(format_key(parse_key(s).unwrap()), s);
        }
    }

    #[test]
    fn shift_is_folded_into_chars() {
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(Key::from(event), key('G'));
    }

    #[test]
    fn defaults_are_valid() {
        assert!(Keymap::with_overrides(&HashMap::new()).is_ok());
    }

    /// The README's key tables say what the keys do. Each row is listed
    /// here with the actions it documents; the keys must run them, and a
    /// row added to the README must be added here.
    #[test]
    fn readme_key_tables_match_the_keymap() {
        use Action::*;
        let page: &[(&str, &[Action])] = &[
            ("`↑` `↓` (`j` `k`)", &[Up, Down]),
            ("`←` `→` (`h` `l`)", &[PrevTab, NextTab]),
            ("`Enter`", &[Open]),
            ("`Esc` `⌫` `alt-←`", &[Back]),
            ("`alt-→`", &[Forward]),
            ("`PgUp` `PgDn`", &[PageUp, PageDown]),
            ("`Home` `End` (`g` `G`)", &[Top, Bottom]),
            ("`1`–`4`", &[Tab1, Tab4]),
            ("`Space` `?`", &[Menu]),
            ("`/`", &[Search]),
            ("`f`", &[FindFile]),
            ("`b`", &[Branch]),
            ("`c`", &[Comment]),
            ("`s`", &[Star]),
            ("`o`", &[OpenInBrowser]),
            ("`y`", &[Copy]),
            ("`i`", &[Hints]),
            ("`I`", &[HintsBrowser]),
            ("`u`", &[UpLevel]),
            ("`H`", &[GoHome]),
            ("`r`", &[Refresh]),
            ("`:` `ctrl-k`", &[CommandPalette]),
            ("`q` `ctrl-c`", &[Quit]),
        ];
        let diff: &[(&str, &[Action])] = &[
            ("`n` `p`", &[NextHunk, PrevHunk]),
            ("`N` `P`", &[NextThread, PrevThread]),
            ("`⇧↓` `⇧↑`", &[NextFile, PrevFile]),
            ("`⇥`", &[SwitchPane]),
            ("`U`", &[NextUnviewed]),
            ("`v`", &[ToggleViewed]),
            ("`m`", &[MarkReviewed]),
            ("`c`", &[Comment]),
            ("`C`", &[FileComment]),
            ("`R`", &[ResolveThread]),
            ("`x`", &[VisualLines]),
            ("`e`", &[ExpandContext]),
            ("`F`", &[FullFile]),
            ("`S`", &[ToggleSplit]),
            ("`t`", &[ToggleTree]),
            ("`M`", &[JumpMove]),
            ("`w`", &[IgnoreWhitespace]),
            ("`a`", &[SubmitReview]),
            ("`Delete`", &[DeleteDraft]),
            ("`ctrl-z`", &[UndoDelete]),
        ];
        let notation = |shown: &str| match shown {
            "↑" => "<Up>".to_owned(),
            "↓" => "<Down>".to_owned(),
            "←" => "<Left>".to_owned(),
            "→" => "<Right>".to_owned(),
            "⌫" => "<BS>".to_owned(),
            "⇥" => "<Tab>".to_owned(),
            "⇧↓" => "<S-Down>".to_owned(),
            "⇧↑" => "<S-Up>".to_owned(),
            "PgUp" => "<PageUp>".to_owned(),
            "PgDn" => "<PageDown>".to_owned(),
            "alt-←" => "<A-Left>".to_owned(),
            "alt-→" => "<A-Right>".to_owned(),
            "ctrl-k" => "<C-k>".to_owned(),
            "ctrl-c" => "<C-c>".to_owned(),
            "ctrl-z" => "<C-z>".to_owned(),
            s if s.chars().count() > 1 => format!("<{s}>"),
            s => s.to_owned(),
        };
        let readme = include_str!("../../../README.md");
        let keys = readme
            .split_once("\n## Keys\n")
            .and_then(|(_, rest)| rest.split_once("\n## Configuration"))
            .map(|(keys, _)| keys)
            .unwrap();
        let (page_table, diff_table) = keys.split_once("Reviewing,").unwrap();
        let keymap = Keymap::default();
        for (table, documented, scope) in [
            (page_table, page, Scope::Page),
            (diff_table, diff, Scope::Diff),
        ] {
            let rows: Vec<&str> = table
                .lines()
                .filter(|l| l.starts_with("| `"))
                .filter_map(|l| l.split('|').nth(1))
                .map(str::trim)
                .collect();
            let listed: Vec<&str> = documented.iter().map(|(k, _)| *k).collect();
            assert_eq!(rows, listed, "README rows and this test's list differ");
            for (cell, actions) in documented {
                for shown in cell.split('`').skip(1).step_by(2) {
                    let key = parse_key(&notation(shown)).unwrap();
                    let does = keymap.resolve(key, scope);
                    assert!(
                        actions.iter().any(|a| does == Some(*a)),
                        "README says `{shown}` does {actions:?}; it does {does:?}"
                    );
                }
            }
        }
    }

    /// What terminals send for Shift-Tab matches `<S-Tab>`.
    #[test]
    fn shift_tab_matches_what_terminals_send() {
        let event = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(parse_key("<S-Tab>").unwrap(), Key::from(event));
        assert_eq!(
            parse_key("<C-S-Tab>").unwrap(),
            Key::new(KeyCode::BackTab, KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn resolves_defaults() {
        let keymap = Keymap::default();
        assert_eq!(keymap.resolve(key('j'), Scope::Page), Some(Action::Down));
        assert_eq!(keymap.resolve(key('g'), Scope::Page), Some(Action::Top));
        assert_eq!(keymap.resolve(key('z'), Scope::Page), None);
    }

    #[test]
    fn every_key_means_one_thing_everywhere() {
        let keymap = Keymap::default();
        for (i, (a, action_a, _)) in keymap.bindings.iter().enumerate() {
            for (b, action_b, _) in &keymap.bindings[i + 1..] {
                assert!(
                    a != b,
                    "{} is both {} and {}",
                    format_key(*a),
                    action_a.name(),
                    action_b.name()
                );
            }
        }
        // The verbs do the same on a page and in the diff.
        for c in ['f', 'c', 'o', 'y', '/', 'u'] {
            assert_eq!(
                keymap.resolve(key(c), Scope::Page),
                keymap.resolve(key(c), Scope::Diff),
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
        assert_eq!(pretty(parse_key("<Enter>").unwrap()), "↵");
        assert_eq!(pretty(parse_key("<C-d>").unwrap()), "ctrl-d");
        assert_eq!(pretty(parse_key("<Space>").unwrap()), "space");
        assert_eq!(pretty(parse_key("G").unwrap()), "G");
    }

    #[test]
    fn overrides_replace_defaults() {
        let overrides = HashMap::from([("down".to_owned(), vec!["z".to_owned()])]);
        let keymap = Keymap::with_overrides(&overrides).unwrap();
        assert_eq!(keymap.resolve(key('z'), Scope::Page), Some(Action::Down));
        assert_eq!(keymap.resolve(key('j'), Scope::Page), None);
        assert!(keymap.keys_in(Action::Down, Scope::Page).eq([key('z')]));
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
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        fn key() -> impl Strategy<Value = Key> {
            let named =
                prop::sample::select(NAMED.iter().map(|(_, code)| *code).collect::<Vec<_>>());
            let mods = prop::sample::select(vec![
                KeyModifiers::NONE,
                KeyModifiers::CONTROL,
                KeyModifiers::ALT,
                KeyModifiers::CONTROL | KeyModifiers::ALT,
            ]);
            prop_oneof![
                // Plain characters, anything printable.
                any::<char>()
                    .prop_filter("printable", |c| !c.is_control())
                    .prop_map(|c| Key::new(KeyCode::Char(c), KeyModifiers::NONE)),
                // Modified letters (notation folds case).
                ("[a-z0-9]", mods.clone()).prop_map(|(c, m)| {
                    Key::new(KeyCode::Char(c.chars().next().unwrap_or('a')), m)
                }),
                (named, mods).prop_map(|(code, m)| Key::new(code, m)),
            ]
        }

        proptest! {
            /// Writing a key down and reading it back gives the same key.
            #[test]
            fn keys_round_trip(key in key()) {
                let written = format_key(key);
                prop_assert_eq!(parse_key(&written), Ok(key), "{}", written);
            }

            /// Any accepted spelling settles on one canonical form.
            #[test]
            fn parsing_settles(s in "<[A-Za-z-]{1,8}>|[ -~]") {
                if let Ok(key) = parse_key(&s) {
                    let canonical = format_key(key);
                    let again = parse_key(&canonical).map(format_key);
                    prop_assert_eq!(again, Ok(canonical));
                }
            }
        }
    }
}
