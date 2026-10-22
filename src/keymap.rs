//! Rebindable keymap. A binding is one key: `c`, `M`, `]`, `<C-s>`,
//! `<Enter>`. A binding belongs to a context; what's on screen makes a
//! stack of contexts, and the innermost that binds the key wins.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

words! {
    /// Where a binding applies.
    #[derive(Hash)]
    pub enum Context {
        /// Its name in config: `diff:x` binds `x` in the diff.
        pub fn name,
        /// Where it is, for "works … only".
        fn place,
    }
    /// Everywhere.
    Global   "global"   "everywhere";
    /// Any page (not the diff).
    Page     "page"     "on pages";
    /// A list with a GitHub filter: issues, pull requests, search results.
    List     "list"     "on lists";
    /// A file or its blame.
    File     "file"     "in files";
    /// A job's log.
    Log      "log"      "in job logs";
    /// A project: its board or table, or an item.
    Project  "project"  "in projects";
    /// A repository's Settings tab.
    Settings "settings" "in a repository's settings";
    /// A pull request's (or commit's) files: the diff and its file tree.
    Diff     "diff"     "in Files changed";
    /// The diff's file tree, when it has the focus.
    Tree     "tree"     "in Files changed's file tree";
}

/// The stacks of contexts that can be active at once, innermost first.
const STACKS: &[&[Context]] = {
    use Context::*;
    &[
        &[List, Page, Global],
        &[File, Page, Global],
        &[Log, Page, Global],
        &[Project, Page, Global],
        &[Settings, Page, Global],
        &[Page, Global],
        &[Tree, Diff, Global],
        &[Diff, Global],
    ]
};

impl Context {
    /// The contexts active when this is the innermost.
    pub fn stack(self) -> &'static [Context] {
        STACKS
            .iter()
            .copied()
            .find(|s| s.first() == Some(&self))
            .unwrap_or(&[Context::Global])
    }
}

/// Declares [`Action`] from one table: config name, default keys, home
/// context and description. A default key may name its own context
/// (`file:V`).
macro_rules! actions {
    ($($action:ident $name:literal [$($key:literal),*] $context:ident $description:literal;)*) => {
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

            /// Where its keys apply unless they say otherwise.
            pub fn context(self) -> Context {
                match self {
                    $(Action::$action => Context::$context,)*
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
    Down              "down"              ["<Down>"]               Global "Move down";
    Up                "up"                ["<Up>"]                 Global "Move up";
    PageDown          "page_down"         ["<PageDown>"]           Global "Page down";
    PageUp            "page_up"           ["<PageUp>"]             Global "Page up";
    Top               "top"               ["<Home>"]               Global "Go to the top";
    Bottom            "bottom"            ["<End>"]                Global "Go to the bottom";
    Open              "open"              ["<Enter>"]              Global "Open the selection";
    Cancel            "cancel"            ["<Esc>"]                Global "Cancel a search or a selection; with nothing to cancel, go back";
    Back              "back"              ["<BS>", "<A-Left>"]     Global "Back";
    Forward           "forward"           ["<A-Right>"]            Global "Forward";
    UpLevel           "up_level"          ["u"]                    Global "Up a level";
    NextTab           "next_page_tab"     ["<Tab>"]                Global "Next tab of the page";
    PrevTab           "prev_page_tab"     ["<S-Tab>"]              Global "Previous tab of the page";
    Tab1              "page_tab_1"        ["1"]                    Global "First tab of the page";
    Tab2              "page_tab_2"        ["2"]                    Global "Second tab of the page";
    Tab3              "page_tab_3"        ["3"]                    Global "Third tab of the page";
    Tab4              "page_tab_4"        ["4"]                    Global "Fourth tab of the page";
    Tab5              "page_tab_5"        ["5"]                    Global "Fifth tab of the page";
    Tab6              "page_tab_6"        ["6"]                    Global "Sixth tab of the page";
    Tab7              "page_tab_7"        ["7"]                    Global "Seventh tab of the page";
    Tab8              "page_tab_8"        ["8"]                    Global "Eighth tab of the page";
    Tab9              "page_tab_9"        ["9"]                    Global "Ninth tab of the page";
    NextOpenTab       "next_open_tab"     ["<C-PageDown>"]         Global "Next open tab";
    PrevOpenTab       "prev_open_tab"     ["<C-PageUp>"]           Global "Previous open tab";
    OpenInTab         "open_in_tab"       ["T"]                    Global "Open the selection in a new tab";
    CloseTab          "close_tab"         ["<C-w>"]                Global "Close the tab";
    OpenTab1          "open_tab_1"        ["<A-1>"]                Global "Go to open tab 1";
    OpenTab2          "open_tab_2"        ["<A-2>"]                Global "Go to open tab 2";
    OpenTab3          "open_tab_3"        ["<A-3>"]                Global "Go to open tab 3";
    OpenTab4          "open_tab_4"        ["<A-4>"]                Global "Go to open tab 4";
    OpenTab5          "open_tab_5"        ["<A-5>"]                Global "Go to open tab 5";
    OpenTab6          "open_tab_6"        ["<A-6>"]                Global "Go to open tab 6";
    OpenTab7          "open_tab_7"        ["<A-7>"]                Global "Go to open tab 7";
    OpenTab8          "open_tab_8"        ["<A-8>"]                Global "Go to open tab 8";
    OpenTab9          "open_tab_9"        ["<A-9>"]                Global "Go to open tab 9";
    GoHome            "go_home"           ["h"]                    Global "Go home";
    Notifications     "notifications"     ["<C-n>"]                Global "Your notifications";
    ReviewGlance      "review_glance"     []                       Global "The pull request's review at a glance";
    Explore           "explore"           []                       Global "Explore: new repositories and topics";
    Offline           "offline"           []                       Global "Offline: the writes queued for GitHub, and the repositories kept offline";
    Search            "search"            ["/"]                    Global "Search what's on screen (on a list, filter it)";
    SearchNext        "search_next"       ["n"]                    Global "Next search match";
    SearchPrev        "search_prev"       ["N"]                    Global "Previous search match";
    SearchWord        "search_word"       []                       Global "Search the word under the cursor";
    SearchGithub      "search_github"     []                       Global "Search GitHub or go to a page";
    FindFile          "find_file"         ["f"]                    Global "Go to file";
    Comment           "comment"           ["c"]                    Global "Comment (on a thread, reply)";
    Copy              "copy_link"         ["l"]                    Global "Copy the link (a queued write: its text)";
    OpenInBrowser     "open_in_browser"   ["o"]                    Global "Open on GitHub";
    OpenLink          "open_link"         []                       Global "Open the link under the cursor in the browser";
    Refresh           "refresh"           ["r"]                    Global "Refresh";
    Menu              "actions_menu"      ["<Space>"]              Global "Everything you can do here";
    CommandPalette    "command_palette"   [":", "<C-k>"]           Global "Command palette: go anywhere, search GitHub, run anything";
    Messages          "messages"          []                       Global "Recent messages and errors";
    Merge             "merge"             ["M"]                    Global "Merge the pull request";
    Approve           "approve"           ["A"]                    Global "Approve the pull request";
    ReadyForReview    "ready_for_review"  []                       Global "Mark the draft ready for review, or convert it to a draft";
    UpdateBranch      "update_branch"     []                       Global "Update the branch from its base";
    Close             "close"             ["x"]                    Global "Close (as a duplicate too) or reopen (an alert: dismiss it); on a running run, cancel it";
    Rerun             "rerun"             ["R"]                    Global "Re-run the workflow run or job (a refused write: send it again)";
    EditComment       "edit"              ["e"]                    Global "Edit the comment or description (a wiki page, your gist, a release, a label, a milestone, a file)";
    DeleteComment     "delete_comment"    ["D"]                    Global "Delete the comment (your gist, a release or its asset, a package's version, your list, a label, a milestone, a branch, a tag, a file)";
    React             "react"             ["+"]                    Global "React to the comment, issue or pull request";
    Checkout          "checkout"          []                       Global "Check out the pull request in your clone (an issue: a new branch for it)";
    Watch             "watch"             []                       Global "Subscribe to or ignore the thread; watch the repository";
    Delete            "delete"            ["<Delete>"]             Global "Delete the selection (a draft comment, a Home section, a write waiting for GitHub)";
    UndoDelete        "undo_delete"       ["<C-z>"]                Global "Bring back what was just deleted";
    Quit              "quit"              ["q"]                    Global "Quit";

    NextSection       "next_section"      ["]", "tree:]"]          Page   "Next section (a box, a Home section, a folder)";
    PrevSection       "prev_section"      ["[", "tree:["]          Page   "Previous section";
    NextCommit        "next_commit"       []                       Page   "Next commit of the pull request";
    PrevCommit        "prev_commit"       []                       Page   "Previous commit of the pull request";
    QuoteReply        "quote_reply"       [">"]                    Page   "Reply quoting the comment";
    Star              "star"              []                       Page   "Star / unstar";
    EditTitle         "edit_title"        []                       Page   "Edit the title";
    EditLabels        "edit_labels"       []                       Page   "Edit the labels";
    EditAssignees     "edit_assignees"    []                       Page   "Edit the assignees";
    RequestReviews    "request_reviews"   []                       Page   "Ask for reviews";
    EditMilestone     "edit_milestone"    []                       Page   "Set the milestone";
    NewIssue          "new_issue"         []                       Page   "New issue";
    NewPr             "new_pull_request"  []                       Page   "New pull request";
    Create            "create"            ["<C-s>"]                Page   "Create or save the form on screen (an issue, a release, a label, a file…)";
    PushBranch        "push_branch"       []                       Page   "Push the branch to GitHub";
    MarkAnswer        "mark_answer"       []                       Page   "Mark the comment as the discussion's answer, or unmark it";
    NewRelease        "new_release"       []                       Page   "New release";
    UploadAsset       "upload_asset"      []                       Page   "Upload an asset to the release";
    NewGist           "new_gist"          []                       Page   "New gist";
    NewWikiPage       "new_wiki_page"     []                       Page   "New wiki page";
    RunWorkflow       "run_workflow"      []                       Page   "Run the workflow by hand";
    Lists             "lists"             []                       Page   "Add the repository to your starred lists, or take it out";
    NewHere           "new_here"          []                       Page   "New here: a label, a milestone, a branch, a tag, a file";
    Fork              "fork"              []                       Page   "Fork the repository";
    NewRepo           "new_repository"    []                       Page   "New repository";
    Branch            "branch"            ["B"]                    Page   "Switch branches or tags";
    DeleteBranch      "delete_branch"     []                       Page   "Delete the pull request's branch, or restore it";
    ChangeBase        "change_base"       []                       Page   "Change the pull request's base branch";
    Revert            "revert"            []                       Page   "Revert the merged pull request";
    Lock              "lock"              []                       Page   "Lock the conversation, or unlock it";
    Pin               "pin"               []                       Page   "Pin the issue, or unpin it";
    Transfer          "transfer"          []                       Page   "Transfer the issue to another repository";
    HideComment       "hide_comment"      []                       Page   "Hide the comment, or unhide it";
    SubIssues         "sub_issues"        []                       Page   "Add or remove sub-issues";
    BlockedBy         "blocked_by"        []                       Page   "Mark the issue blocked by another, or no longer";
    NewDiscussion     "new_discussion"    []                       Page   "New discussion";
    KeepOffline       "keep_offline"      []                       Page   "Keep the repository to read offline, or stop";
    Hints             "hints"             ["i", "diff:i"]          Page   "Follow a link by its letters";
    HintsBrowser      "hints_browser"     []                       Page   "Follow a link, in the browser";
    ToggleState       "toggle_state"      ["t"]                    List   "Open / closed / all (in the inbox, unread / read)";
    Sort              "sort"              ["S"]                    List   "Change the sort";
    MarkRead          "mark_read"         ["m"]                    List   "Mark the notification read";
    MarkDone          "mark_done"         ["d"]                    List   "Mark the notification done";
    MarkAllRead       "mark_all_read"     []                       List   "Mark everything shown read";
    MarkAllDone       "mark_all_done"     []                       List   "Mark everything shown done";
    SaveSection       "save_to_home"      []                       List   "Save this list as a Home section";
    RenameSection     "rename_section"    []                       Page   "Rename the Home section";
    MoveSectionUp     "move_section_up"   []                       Page   "Move the Home section up";
    MoveSectionDown   "move_section_down" []                       Page   "Move the Home section down";
    Blame             "blame"             ["b"]                    File   "Blame the file at this line, or back to the file";
    ScrollLeft        "scroll_left"       ["<Left>", "log:<Left>", "diff:<Left>"] File "Scroll left";
    ScrollRight       "scroll_right"      ["<Right>", "log:<Right>", "diff:<Right>"] File "Scroll right";
    WordNext          "word_next"         ["<C-Right>", "log:<C-Right>", "diff:<C-Right>"] File "Next word (to search or go to its definition)";
    WordPrev          "word_prev"         ["<C-Left>", "log:<C-Left>", "diff:<C-Left>"] File "Previous word";
    Symbols           "symbols"           []                       File   "Symbols in this file, to go to one";
    Definition        "go_to_definition"  ["g", "diff:g"]          File   "Go to the definition of the word under the cursor";
    RepoSymbols       "repo_symbols"      []                       File   "Symbols in the repository, to go to one";
    References        "references"        []                       File   "Where the word under the cursor is used";
    PrevColumn        "prev_column"       ["<Left>"]               Project "Previous column of the board";
    NextColumn        "next_column"       ["<Right>"]              Project "Next column of the board";
    MoveItem          "move_item"         ["m"]                    Project "Move the item to another column";
    SetField          "set_field"         []                       Page   "Set a field of the item in its project";
    AddItem           "add_item"          ["a"]                    Project "Add an issue, a pull request or a draft";
    ArchiveItem       "archive_item"      []                       Project "Archive the item, or restore it";
    AddSetting        "add_setting"       ["a"]                    Settings "Add a branch rule, a collaborator, a webhook, a deploy key, a secret or a variable; publish a Pages site";
    EditSetting       "edit_setting"      ["e"]                    Settings "Edit the rule, webhook, variable, Pages site or custom properties; set a secret; change the collaborator's role";
    DeleteSetting     "delete_setting"    ["D"]                    Settings "Delete the rule, webhook, deploy key, secret, variable or cache; remove the collaborator or invitation; unpublish the site";
    ToggleSetting     "toggle_setting"    ["x"]                    Settings "Turn the ruleset or webhook on or off; archive or unarchive the repository";
    Redeliver         "redeliver"         []                       Settings "Redeliver the webhook delivery";

    NextHunk          "next_change"       ["]"]                    Diff   "Next change";
    PrevHunk          "prev_change"       ["["]                    Diff   "Previous change";
    NextFile          "next_file"         ["<S-Down>"]             Diff   "Next file";
    PrevFile          "prev_file"         ["<S-Up>"]               Diff   "Previous file";
    OldSide           "old_side"          ["<S-Left>"]             Diff   "Old side of a split row (for comments)";
    NewSide           "new_side"          ["<S-Right>"]            Diff   "New side of a split row (for comments)";
    NextUnviewed      "next_unviewed"     ["U"]                    Diff   "Next unviewed file";
    NextThread        "next_thread"       []                       Diff   "Next unresolved thread";
    PrevThread        "prev_thread"       []                       Diff   "Previous unresolved thread";
    JumpMove          "jump_move"         []                       Diff   "Jump to the other end of moved code";
    SwitchPane        "switch_pane"       ["<Tab>"]                Diff   "Switch between the file tree and the diff";
    Fold              "fold"              ["<Left>"]               Tree   "Fold the folder (on a file, go to its folder)";
    Unfold            "unfold"            ["<Right>"]              Tree   "Unfold the folder";
    TreeNarrower      "tree_narrower"     []                       Tree   "Narrow the file tree";
    TreeWider         "tree_wider"        []                       Tree   "Widen the file tree";
    FoldToggle        "fold_toggle"       ["z"]                    Diff   "Fold or unfold the file, thread or review";
    FoldAll           "fold_all"          ["Z"]                    Diff   "Fold every file";
    UnfoldAll         "unfold_all"        []                       Diff   "Unfold every file";
    ToggleTree        "toggle_tree"       ["\\"]                   Diff   "Show or hide the file tree";
    ToggleSplit       "toggle_split"      ["|"]                    Diff   "Split or unified view";
    WrapLines         "wrap_lines"        ["w"]                    Diff   "Wrap long lines";
    IgnoreWhitespace  "ignore_whitespace" ["W"]                    Diff   "Ignore whitespace changes";
    BySyntax          "by_syntax"         ["y"]                    Diff   "Compare by syntax or by lines";
    FullFile          "full_file"         ["F"]                    Diff   "Show the whole file";
    ExpandContext     "expand_context"    ["E"]                    Diff   "Show more context";
    ToggleSinceReview "since_review"      []                       Diff   "Show only changes since your last review";
    PickCommits       "pick_commits"      []                       Diff   "Choose commits to view";
    ToggleViewed      "toggle_viewed"     ["v"]                    Diff   "Mark the file viewed (syncs with GitHub)";
    MarkReviewed      "mark_reviewed"     ["m"]                    Diff   "Mark the change reviewed";
    VisualLines       "select_lines"      ["s", "file:s"]          Diff   "Select lines";
    Suggest           "suggest"           []                       Diff   "Suggest a change (opens $EDITOR)";
    FileComment       "file_comment"      []                       Diff   "Comment on the whole file";
    ResolveThread     "resolve"           ["x"]                    Diff   "Resolve or unresolve the thread";
    SubmitReview      "submit_review"     ["<C-s>"]                Diff   "Submit your review";
}

impl Action {
    /// `alt-1`…`alt-9`, in order.
    pub const OPEN_TABS: [Action; 9] = {
        use Action::*;
        [
            OpenTab1, OpenTab2, OpenTab3, OpenTab4, OpenTab5, OpenTab6, OpenTab7, OpenTab8,
            OpenTab9,
        ]
    };

    /// The page's tabs 1…9, in order.
    pub const PAGE_TABS: [Action; 9] = {
        use Action::*;
        [Tab1, Tab2, Tab3, Tab4, Tab5, Tab6, Tab7, Tab8, Tab9]
    };

    pub fn from_name(name: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|a| a.name() == name)
    }

    /// Where it works by default, for when it's used elsewhere.
    pub fn not_here(self) -> String {
        let mut places: Vec<&str> = Vec::new();
        for key in self.defaults() {
            let place = scoped(key, self.context()).1.place();
            if !places.contains(&place) {
                places.push(place);
            }
        }
        if places.is_empty() {
            places.push(self.context().place());
        }
        let places = match places.split_last() {
            Some((last, [])) => (*last).to_owned(),
            Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
            None => String::new(),
        };
        format!("{}: works {places}", self.description())
    }

    /// Moving around the screen, rather than doing something.
    pub fn moves(self) -> bool {
        use Action::*;
        matches!(
            self,
            Down | Up
                | PageDown
                | PageUp
                | NextSection
                | PrevSection
                | WordNext
                | WordPrev
                | ScrollLeft
                | ScrollRight
                | Top
                | Bottom
                | Open
                | Cancel
        )
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
    /// Tab with Shift). Ctrl-i is Tab, as terminals send it.
    pub fn new(code: KeyCode, mods: KeyModifiers) -> Self {
        let (code, mods) = match code {
            KeyCode::Char('i') if mods == KeyModifiers::CONTROL => {
                (KeyCode::Tab, KeyModifiers::NONE)
            }
            KeyCode::Tab if mods.contains(KeyModifiers::SHIFT) => (KeyCode::BackTab, mods),
            code => (code, mods),
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

/// Keys by name: as the config spells them, and as people write them.
const NAMED: &[(&str, KeyCode, &str)] = &[
    ("Enter", KeyCode::Enter, "↵"),
    ("CR", KeyCode::Enter, "↵"),
    ("Esc", KeyCode::Esc, "esc"),
    ("Tab", KeyCode::Tab, "⇥"),
    ("S-Tab", KeyCode::BackTab, "⇧⇥"),
    ("BS", KeyCode::Backspace, "⌫"),
    ("Space", KeyCode::Char(' '), "space"),
    ("Up", KeyCode::Up, "↑"),
    ("Down", KeyCode::Down, "↓"),
    ("Left", KeyCode::Left, "←"),
    ("Right", KeyCode::Right, "→"),
    ("PageUp", KeyCode::PageUp, "pgup"),
    ("PageDown", KeyCode::PageDown, "pgdn"),
    ("Home", KeyCode::Home, "home"),
    ("End", KeyCode::End, "end"),
    ("Delete", KeyCode::Delete, "del"),
    ("Del", KeyCode::Delete, "del"),
    ("lt", KeyCode::Char('<'), "<"),
];

/// Parses one key: `c`, `]`, `<C-s>`, `<Enter>`. More than one key (`gg`)
/// is refused: bindings are single keys.
pub fn parse_key(s: &str) -> Result<Key, String> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (None, _) => Err("empty key".into()),
        (Some(c), None) => Ok(Key::new(KeyCode::Char(c), KeyModifiers::NONE)),
        _ => match s.strip_prefix('<').and_then(|k| k.strip_suffix('>')) {
            Some(name) => parse_named(name).ok_or_else(|| format!("unknown key `{s}`")),
            None => Err(format!(
                "`{s}` is more than one key: bind one key, like `x` or `<C-x>`"
            )),
        },
    }
}

fn parse_named(name: &str) -> Option<Key> {
    if let Some((_, code, _)) = NAMED.iter().find(|(n, ..)| n.eq_ignore_ascii_case(name)) {
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
            .find(|(n, ..)| n.eq_ignore_ascii_case(rest))
            .map(|(_, code, _)| *code)?,
    };
    Some(Key::new(code, mods))
}

/// Writes `key` as the config spells it.
pub fn format_key(key: Key) -> String {
    let base = match key.code {
        KeyCode::Char('<') => "lt".to_owned(),
        KeyCode::Char(' ') => "Space".to_owned(),
        KeyCode::Char(c) if key.mods.is_empty() => return c.to_string(),
        KeyCode::Char(c) => c.to_string(),
        code => NAMED
            .iter()
            .find(|(_, c, _)| *c == code)
            .map_or_else(|| format!("{code:?}"), |(n, ..)| (*n).to_owned()),
    };
    let mods = [
        (KeyModifiers::CONTROL, "C-"),
        (KeyModifiers::ALT, "A-"),
        (KeyModifiers::SHIFT, "S-"),
    ];
    let prefix: String = (mods.into_iter())
        .filter_map(|(m, said)| key.mods.contains(m).then_some(said))
        .collect();
    format!("<{prefix}{base}>")
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub key: Key,
    pub action: Action,
    pub context: Context,
}

#[derive(Debug, Clone)]
pub struct Keymap {
    bindings: Vec<Binding>,
}

impl Default for Keymap {
    /// The default bindings (a test checks they're valid).
    fn default() -> Self {
        Self::with_overrides(&HashMap::new()).unwrap_or(Self {
            bindings: Vec::new(),
        })
    }
}

/// `diff:x` binds `x` in the diff; a plain key applies in `home`.
fn scoped(key: &str, home: Context) -> (&str, Context) {
    match key.split_once(':') {
        Some((name, rest)) if !rest.is_empty() => Context::ALL
            .into_iter()
            .find(|c| c.name() == name)
            .map_or((key, home), |c| (rest, c)),
        _ => (key, home),
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
            let keys: Vec<&str> = match overrides.get(action.name()) {
                Some(list) => list.iter().map(String::as_str).collect(),
                None => action.defaults().to_vec(),
            };
            for key in keys {
                let (key, context) = scoped(key, action.context());
                let key = parse_key(key).map_err(|e| format!("[keys] {}: {e}", action.name()))?;
                bindings.push(Binding {
                    key,
                    action,
                    context,
                });
            }
        }
        let keymap = Self { bindings };
        keymap.validate()?;
        Ok(keymap)
    }

    /// Rejects two bindings of one key in one context.
    fn validate(&self) -> Result<(), String> {
        for (i, a) in self.bindings.iter().enumerate() {
            for b in self.bindings.iter().skip(i + 1) {
                if a.key == b.key && a.context == b.context {
                    return Err(format!(
                        "key `{}` ({}) conflicts with `{}` ({})",
                        format_key(a.key),
                        a.action.name(),
                        format_key(b.key),
                        b.action.name()
                    ));
                }
            }
        }
        Ok(())
    }

    /// The bindings active in `stack`, innermost context first; one
    /// shadowed by an inner binding of the same key is left out.
    fn active<'a>(&'a self, stack: &'a [Context]) -> impl Iterator<Item = &'a Binding> {
        let mut seen: Vec<Key> = Vec::new();
        stack
            .iter()
            .flat_map(|c| self.bindings.iter().filter(move |b| b.context == *c))
            .filter(move |b| {
                let fresh = !seen.contains(&b.key);
                if fresh {
                    seen.push(b.key);
                }
                fresh
            })
    }

    /// What `key` does in `stack`.
    pub fn resolve(&self, key: Key, stack: &[Context]) -> Option<Action> {
        self.active(stack).find(|b| b.key == key).map(|b| b.action)
    }

    /// What `key` does in a context not active now, to say where it works.
    pub fn elsewhere(&self, key: Key) -> Option<Action> {
        self.bindings
            .iter()
            .find(|b| b.key == key)
            .map(|b| b.action)
    }

    /// The keys that run `action` in `stack`.
    pub fn keys_in<'a>(
        &'a self,
        action: Action,
        stack: &'a [Context],
    ) -> impl Iterator<Item = Key> + 'a {
        self.active(stack)
            .filter(move |b| b.action == action)
            .map(|b| b.key)
    }
}

/// A key as people write it: `↵`, `esc`, `⇥`, `space`, `ctrl-d`.
pub fn pretty(key: Key) -> String {
    let named = NAMED.iter().find(|(_, c, _)| *c == key.code);
    let base = match (key.code, named) {
        (_, Some((.., said))) => (*said).to_owned(),
        (KeyCode::F(n), _) => format!("f{n}"),
        (KeyCode::Char(c), _) => c.to_string(),
        (code, _) => format!("{code:?}").to_lowercase(),
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
/// The README's key tables: written from the default keymap, so they
/// can't say anything the keys don't do (a test compares them).
pub fn readme_tables() -> String {
    let keymap = Keymap::default();
    let mut out = String::new();
    for context in Context::ALL {
        let keys_of = |action: Action| -> Vec<String> {
            (keymap.bindings.iter())
                .filter(|b| b.action == action && b.context == context)
                // A table cell's `|` is escaped, even in code.
                .map(|b| format!("`{}`", pretty(b.key).replace('|', "\\|")))
                .collect()
        };
        let rest = |tabs: &[Action; 9], a: &Action| tabs.get(1..).unwrap_or_default().contains(a);
        let rows: Vec<(String, &str)> = Action::ALL
            .iter()
            .filter(|a| !rest(&Action::OPEN_TABS, a) && !rest(&Action::PAGE_TABS, a))
            .filter_map(|&action| {
                let mut keys = keys_of(action);
                // Tabs 1 to 9 are one row.
                let tabs = match action {
                    Action::OpenTab1 => Some((Action::OpenTab9, "Go to open tab 1 to 9")),
                    Action::Tab1 => Some((Action::Tab9, "Go to the page's tab 1 to 9")),
                    _ => None,
                };
                if let Some((last, what)) = tabs.filter(|_| !keys.is_empty()) {
                    keys.extend(keys_of(last));
                    return Some((keys.join("–"), what));
                }
                (!keys.is_empty()).then(|| (keys.join(" "), action.description()))
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        let place = context.place();
        let mut title = place.chars();
        let title: String = title
            .next()
            .map(|c| c.to_uppercase().chain(title).collect())
            .unwrap_or_default();
        let width = rows
            .iter()
            .map(|(k, _)| k.chars().count())
            .max()
            .unwrap_or(0)
            .max(4);
        out.push_str(&format!("**{title}** (`{}:`)\n\n", context.name()));
        out.push_str(&format!("| {:width$} | Action |\n", "Keys"));
        out.push_str(&format!("| {} | ------ |\n", "-".repeat(width)));
        for (keys, what) in rows {
            let pad = width.saturating_sub(keys.chars().count());
            out.push_str(&format!("| {keys}{} | {what} |\n", " ".repeat(pad)));
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(c: char) -> Key {
        Key::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    fn k(s: &str) -> Key {
        parse_key(s).unwrap()
    }

    const PAGE: &[Context] = &[Context::Page, Context::Global];
    const LIST: &[Context] = &[Context::List, Context::Page, Context::Global];
    const DIFF: &[Context] = &[Context::Diff, Context::Global];
    const TREE: &[Context] = &[Context::Tree, Context::Diff, Context::Global];

    #[test]
    fn parses_one_key() {
        assert_eq!(k("G"), key('G'));
        assert_eq!(
            k("<C-s>"),
            Key::new(KeyCode::Char('s'), KeyModifiers::CONTROL)
        );
        assert_eq!(k("<c-S>"), k("<C-s>"));
        assert_eq!(k("<lt>"), key('<'));
        assert_eq!(k("<"), key('<'));
        assert_eq!(k("<C-i>"), k("<Tab>"));
        assert_eq!(k("<S-Down>"), Key::new(KeyCode::Down, KeyModifiers::SHIFT));
        for bad in ["gg", "]c", "<C-w>j", "<Nope>", "<C-d", ""] {
            parse_key(bad).unwrap_err();
        }
        assert!(parse_key("gg").unwrap_err().contains("more than one key"));
    }

    #[test]
    fn formats_round_trip() {
        for s in [
            "G", "<C-s>", "<Enter>", "<Esc>", "<S-Tab>", "<lt>", "]", ":", "<S-Down>", "<A-1>",
        ] {
            assert_eq!(format_key(k(s)), s);
        }
    }

    #[test]
    fn shift_is_folded_into_chars() {
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(Key::from(event), key('G'));
        let event = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(k("<S-Tab>"), Key::from(event));
    }

    #[test]
    fn defaults_are_valid() {
        Keymap::with_overrides(&HashMap::new()).unwrap();
    }

    /// The README's key tables are the keymap's.
    #[test]
    fn readme_key_tables_match_the_keymap() {
        let readme = include_str!("../README.md");
        assert!(
            readme.contains(&readme_tables()),
            "README's key tables are out of date; they should read:\n\n{}",
            readme_tables()
        );
    }

    #[test]
    fn inner_contexts_win() {
        let keymap = Keymap::default();
        assert_eq!(keymap.resolve(key('t'), LIST), Some(Action::ToggleState));
        assert_eq!(keymap.resolve(key('t'), PAGE), None);
        assert_eq!(keymap.elsewhere(key('t')), Some(Action::ToggleState));
        assert_eq!(keymap.resolve(key('M'), PAGE), Some(Action::Merge));
        assert_eq!(keymap.resolve(key(']'), DIFF), Some(Action::NextHunk));
        assert_eq!(keymap.resolve(key(']'), TREE), Some(Action::NextSection));
        assert_eq!(keymap.resolve(key(']'), PAGE), Some(Action::NextSection));
        assert_eq!(keymap.resolve(k("<Left>"), DIFF), Some(Action::ScrollLeft));
        assert_eq!(keymap.resolve(k("<Left>"), TREE), Some(Action::Fold));
        assert_eq!(keymap.resolve(k("<Tab>"), PAGE), Some(Action::NextTab));
        assert_eq!(keymap.resolve(k("<Tab>"), DIFF), Some(Action::SwitchPane));
        assert_eq!(keymap.resolve(key('x'), PAGE), Some(Action::Close));
        assert_eq!(keymap.resolve(key('x'), DIFF), Some(Action::ResolveThread));
        assert_eq!(keymap.resolve(key('i'), DIFF), Some(Action::Hints));
        assert_eq!(keymap.resolve(key('i'), PAGE), Some(Action::Hints));
        assert_eq!(keymap.resolve(key('2'), PAGE), Some(Action::Tab2));
        assert!(keymap.keys_in(Action::ScrollLeft, TREE).next().is_none());
    }

    #[test]
    fn rejects_bad_bindings() {
        let reject = |name: &str, key: &str, says: &str| {
            let overrides = HashMap::from([(name.to_owned(), vec![key.to_owned()])]);
            let err = Keymap::with_overrides(&overrides).unwrap_err();
            assert!(err.contains(says), "{name} = {key}: {err}");
        };
        reject("fly", "f", "unknown action");
        reject("down", "<Up>", "conflicts");
        reject("star", "gg", "more than one key");
        reject("star", "<Nope>", "unknown key");
        // An inner context may shadow a global key.
        let shadow = HashMap::from([("star".to_owned(), vec!["page:r".to_owned()])]);
        let keymap = Keymap::with_overrides(&shadow).unwrap();
        assert_eq!(keymap.resolve(key('r'), PAGE), Some(Action::Star));
        assert_eq!(keymap.resolve(key('r'), DIFF), Some(Action::Refresh));
    }

    #[test]
    fn overrides_replace_defaults() {
        let overrides = HashMap::from([("down".to_owned(), vec!["j".to_owned()])]);
        let keymap = Keymap::with_overrides(&overrides).unwrap();
        assert_eq!(keymap.resolve(key('j'), PAGE), Some(Action::Down));
        assert_eq!(keymap.resolve(k("<Down>"), PAGE), None);
        assert!(keymap.keys_in(Action::Down, PAGE).eq([key('j')]));
    }

    #[test]
    fn keys_read_like_people_write_them() {
        assert_eq!(pretty(k("<Enter>")), "↵");
        assert_eq!(pretty(k("<C-s>")), "ctrl-s");
        assert_eq!(pretty(k("<S-Down>")), "⇧↓");
        assert_eq!(
            Action::ScrollLeft.not_here(),
            "Scroll left: works in files, in job logs and in Files changed"
        );
    }

    mod props {
        use super::*;
        use proptest::prelude::*;

        fn key() -> impl Strategy<Value = Key> {
            let named =
                prop::sample::select(NAMED.iter().map(|(_, code, _)| *code).collect::<Vec<_>>());
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
                    prop_assert_eq!(parse_key(&canonical).map(format_key), Ok(canonical));
                }
            }
        }
    }
}
