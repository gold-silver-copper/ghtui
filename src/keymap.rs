//! Rebindable keymap. A binding is a key sequence in vim notation: `j`,
//! `gg`, `]c`, `<C-d>`, `<Enter>`. Digits typed first are a count (`5j`,
//! `3gt`). A binding belongs to a context; what's on screen makes a stack
//! of contexts, and the innermost that binds the keys wins.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Where a binding applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    /// Everywhere.
    Global,
    /// Any page (not the diff).
    Page,
    /// A list with a GitHub filter: issues, pull requests, search results.
    List,
    /// A file or its blame.
    File,
    /// A job's log.
    Log,
    /// A pull request's (or commit's) files: the diff and its file tree.
    Diff,
    /// The diff's file tree, when it has the focus.
    Tree,
    /// A project: its board or table, or an item.
    Project,
}

/// The stacks of contexts that can be active at once, innermost first.
const STACKS: &[&[Context]] = {
    use Context::*;
    &[
        &[List, Page, Global],
        &[File, Page, Global],
        &[Log, Page, Global],
        &[Project, Page, Global],
        &[Page, Global],
        &[Tree, Diff, Global],
        &[Diff, Global],
    ]
};

impl Context {
    const ALL: [Context; 8] = [
        Context::Global,
        Context::Page,
        Context::List,
        Context::File,
        Context::Log,
        Context::Project,
        Context::Diff,
        Context::Tree,
    ];

    /// Its name in config: `diff:x` binds `x` in the diff.
    pub fn name(self) -> &'static str {
        match self {
            Context::Global => "global",
            Context::Page => "page",
            Context::List => "list",
            Context::File => "file",
            Context::Log => "log",
            Context::Diff => "diff",
            Context::Tree => "tree",
            Context::Project => "project",
        }
    }

    /// Where it is, for "works … only".
    fn place(self) -> &'static str {
        match self {
            Context::Global => "everywhere",
            Context::Page => "on pages",
            Context::List => "on lists",
            Context::File => "in files",
            Context::Log => "in job logs",
            Context::Diff => "in Files changed",
            Context::Tree => "in Files changed's file tree",
            Context::Project => "in projects",
        }
    }

    /// The contexts active when this is the innermost.
    pub fn stack(self) -> &'static [Context] {
        STACKS
            .iter()
            .copied()
            .find(|s| s.first() == Some(&self))
            .unwrap_or(&[Context::Global])
    }

    /// Whether the two can be active at once.
    fn meets(self, other: Context) -> bool {
        STACKS
            .iter()
            .any(|stack| stack.contains(&self) && stack.contains(&other))
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
    Down              "down"              ["j", "<Down>", "<C-n>"] Global "Move down";
    Up                "up"                ["k", "<Up>", "<C-p>"]   Global "Move up";
    HalfPageDown      "half_page_down"    ["<C-d>"]                Global "Half page down";
    HalfPageUp        "half_page_up"      ["<C-u>"]                Global "Half page up";
    PageDown          "page_down"         ["<C-f>", "<PageDown>"]  Global "Page down";
    PageUp            "page_up"           ["<C-b>", "<PageUp>"]    Global "Page up";
    ScrollDown        "scroll_down"       ["<C-e>"]                Global "Scroll down a line";
    ScrollUp          "scroll_up"         ["<C-y>"]                Global "Scroll up a line";
    ScrollCenter      "scroll_center"     ["zz"]                   Global "Scroll the cursor to the middle";
    ScrollTop         "scroll_top"        ["zt"]                   Global "Scroll the cursor to the top";
    ScrollBottom      "scroll_bottom"     ["zb"]                   Global "Scroll the cursor to the bottom";
    ScreenTop         "screen_top"        ["H"]                    Global "To the top of the screen";
    ScreenMiddle      "screen_middle"     ["M"]                    Global "To the middle of the screen";
    ScreenBottom      "screen_bottom"     ["L"]                    Global "To the bottom of the screen";
    Top               "top"               ["gg", "<Home>"]         Global "Go to the top (with a count, to that line)";
    Bottom            "bottom"            ["G", "<End>"]           Global "Go to the bottom";
    Open              "open"              ["<Enter>"]              Global "Open the selection";
    Cancel            "cancel"            ["<Esc>"]                Global "Cancel a search, a selection or keys typed so far";
    Back              "back"              ["<C-o>", "<BS>", "<A-Left>"] Global "Back";
    Forward           "forward"           ["<C-i>", "<A-Right>"]   Global "Forward";
    CloseView         "close_view"        ["q"]                    Global "Close this view: back, or close the tab";
    UpLevel           "up_level"          ["-", "u"]               Global "Up a level";
    NextTab           "next_page_tab"     ["gt"]                   Global "Next tab of the page (with a count, that tab)";
    PrevTab           "prev_page_tab"     ["gT"]                   Global "Previous tab of the page";
    Tab1              "page_tab_1"        []                       Global "First tab of the page";
    Tab2              "page_tab_2"        []                       Global "Second tab of the page";
    Tab3              "page_tab_3"        []                       Global "Third tab of the page";
    Tab4              "page_tab_4"        []                       Global "Fourth tab of the page";
    Tab5              "page_tab_5"        []                       Global "Fifth tab of the page";
    Tab6              "page_tab_6"        []                       Global "Sixth tab of the page";
    Tab7              "page_tab_7"        []                       Global "Seventh tab of the page";
    Tab8              "page_tab_8"        []                       Global "Eighth tab of the page";
    Tab9              "page_tab_9"        []                       Global "Ninth tab of the page";
    NextOpenTab       "next_open_tab"     ["]b", "<C-PageDown>"]   Global "Next open tab";
    PrevOpenTab       "prev_open_tab"     ["[b", "<C-PageUp>"]     Global "Previous open tab";
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
    GoHome            "go_home"           ["gh"]                   Global "Go home";
    Notifications     "notifications"     ["gn"]                   Global "Your notifications";
    ReviewGlance      "review_glance"     ["gr"]                   Global "The pull request's review at a glance";
    Explore           "explore"           []                       Global "Explore: new repositories and topics";
    Search            "search"            ["/"]                    Global "Search what's on screen (on a list, filter it)";
    SearchBack        "search_back"       ["?"]                    Global "Search what's on screen, backwards";
    SearchNext        "search_next"       ["n"]                    Global "Next search match";
    SearchPrev        "search_prev"       ["N"]                    Global "Previous search match";
    SearchWord        "search_word"       ["*"]                    Global "Search the word under the cursor";
    SearchWordBack    "search_word_back"  ["#"]                    Global "Search the word under the cursor, backwards";
    SearchGithub      "search_github"     ["g/"]                   Global "Search GitHub or go to a page";
    FindFile          "find_file"         ["f"]                    Global "Go to file";
    Comment           "comment"           ["c"]                    Global "Comment (on a thread, reply)";
    Copy              "copy_link"         ["y"]                    Global "Copy the link";
    OpenInBrowser     "open_in_browser"   ["o"]                    Global "Open on GitHub";
    OpenLink          "open_link"         ["gx"]                   Global "Open the link under the cursor in the browser";
    Refresh           "refresh"           ["r", "<C-l>"]           Global "Refresh";
    Menu              "actions_menu"      ["<Space>"]              Global "Everything you can do here";
    CommandPalette    "command_palette"   [":", "<C-k>"]           Global "Command palette";
    Messages          "messages"          []                       Global "Recent messages and errors";
    Merge             "merge"             [",m"]                   Global "Merge the pull request";
    Approve           "approve"           [",a"]                   Global "Approve the pull request";
    ReadyForReview    "ready_for_review"  [",r"]                   Global "Mark the draft ready for review, or convert it to a draft";
    UpdateBranch      "update_branch"     [",u"]                   Global "Update the branch from its base";
    Close             "close"             [",x"]                   Global "Close (as a duplicate too) or reopen (an alert: dismiss it); on a running run, cancel it";
    Rerun             "rerun"             [",R"]                   Global "Re-run the workflow run or job";
    EditComment       "edit"              [",e"]                   Global "Edit the comment or description (a wiki page, your gist, a release, a label, a milestone, a file)";
    DeleteComment     "delete_comment"    [",D"]                   Global "Delete the comment (your gist, a release or its asset, a package's version, your list, a label, a milestone, a branch, a tag, a file)";
    React             "react"             [",+"]                   Global "React to the comment, issue or pull request";
    Checkout          "checkout"          [",c"]                   Global "Check out the pull request in your clone (an issue: a new branch for it)";
    Watch             "watch"             [",w"]                   Global "Subscribe to or ignore the thread; watch the repository";
    Delete            "delete"            ["<Delete>"]             Global "Delete the selection (a draft comment, a Home section)";
    UndoDelete        "undo_delete"       ["<C-z>"]                Global "Bring back what was just deleted";
    Quit              "quit"              ["ZZ", "<C-c>"]          Global "Quit";

    NextSection       "next_section"      ["]]", "tree:]]"]        Page   "Next section (a box, a Home section, a folder)";
    PrevSection       "prev_section"      ["[[", "tree:[["]        Page   "Previous section";
    NextItem          "next_item"         ["}"]                    Page   "Next comment or row";
    PrevItem          "prev_item"         ["{"]                    Page   "Previous comment or row";
    NextCommit        "next_commit"       ["]C"]                   Page   "Next commit of the pull request";
    PrevCommit        "prev_commit"       ["[C"]                   Page   "Previous commit of the pull request";
    QuoteReply        "quote_reply"       [">"]                    Page   "Reply quoting the comment";
    Star              "star"              [",*"]                   Page   "Star / unstar";
    EditTitle         "edit_title"        [",T"]                   Page   "Edit the title";
    EditLabels        "edit_labels"       [",l"]                   Page   "Edit the labels";
    EditAssignees     "edit_assignees"    [",A"]                   Page   "Edit the assignees";
    RequestReviews    "request_reviews"   [",q"]                   Page   "Ask for reviews";
    EditMilestone     "edit_milestone"    [",M"]                   Page   "Set the milestone";
    NewIssue          "new_issue"         [",i"]                   Page   "New issue";
    NewPr             "new_pull_request"  [",p"]                   Page   "New pull request";
    Create            "create"            [",s"]                   Page   "Create or save the form on screen (an issue, a release, a label, a file…)";
    PushBranch        "push_branch"       [",P"]                   Page   "Push the branch to GitHub";
    MarkAnswer        "mark_answer"       [",!"]                   Page   "Mark the comment as the discussion's answer, or unmark it";
    NewRelease        "new_release"       [",V"]                   Page   "New release";
    UploadAsset       "upload_asset"      [",U"]                   Page   "Upload an asset to the release";
    NewGist           "new_gist"          [",G"]                   Page   "New gist";
    NewWikiPage       "new_wiki_page"     [",W"]                   Page   "New wiki page";
    RunWorkflow       "run_workflow"      [",E"]                   Page   "Run the workflow by hand";
    Lists             "lists"             [",B"]                   Page   "Add the repository to your starred lists, or take it out";
    NewHere           "new_here"          [",o"]                   Page   "New here: a label, a milestone, a branch, a tag, a file";
    Fork              "fork"              [",F"]                   Page   "Fork the repository";
    NewRepo           "new_repository"    [",C"]                   Page   "New repository";
    Branch            "branch"            ["B"]                    Page   "Switch branches or tags";
    DeleteBranch      "delete_branch"     [",b"]                   Page   "Delete the pull request's branch, or restore it";
    ChangeBase        "change_base"       []                       Page   "Change the pull request's base branch";
    Revert            "revert"            []                       Page   "Revert the merged pull request";
    Lock              "lock"              [",L"]                   Page   "Lock the conversation, or unlock it";
    Pin               "pin"               []                       Page   "Pin the issue, or unpin it";
    Transfer          "transfer"          []                       Page   "Transfer the issue to another repository";
    HideComment       "hide_comment"      [",H"]                   Page   "Hide the comment, or unhide it";
    SubIssues         "sub_issues"        [",S"]                   Page   "Add or remove sub-issues";
    BlockedBy         "blocked_by"        [",K"]                   Page   "Mark the issue blocked by another, or no longer";
    NewDiscussion     "new_discussion"    []                       Page   "New discussion";
    Hints             "hints"             ["i", "diff:i"]          Page   "Follow a link by its letters";
    HintsBrowser      "hints_browser"     ["I", "diff:I"]          Page   "Follow a link, in the browser";
    ToggleState       "toggle_state"      ["t"]                    List   "Open / closed / all (in the inbox, unread / read)";
    Sort              "sort"              ["S"]                    List   "Change the sort";
    MarkRead          "mark_read"         [",n"]                   List   "Mark the notification read";
    MarkDone          "mark_done"         [",d"]                   List   "Mark the notification done";
    MarkAllRead       "mark_all_read"     [",N"]                   List   "Mark everything shown read";
    MarkAllDone       "mark_all_done"     [",D"]                   List   "Mark everything shown done";
    SaveSection       "save_to_home"      []                       List   "Save this list as a Home section";
    RenameSection     "rename_section"    []                       Page   "Rename the Home section";
    MoveSectionUp     "move_section_up"   ["<A-Up>"]               Page   "Move the Home section up";
    MoveSectionDown   "move_section_down" ["<A-Down>"]             Page   "Move the Home section down";
    Blame             "blame"             ["gb"]                   File   "Blame the file at this line, or back to the file";
    ScrollLeft        "scroll_left"       ["h", "log:h", "diff:h"] File   "Scroll left";
    ScrollRight       "scroll_right"      ["l", "log:l", "diff:l"] File   "Scroll right";
    WordNext          "word_next"         ["w", "log:w", "diff:w"] File   "Next word (for * and gx)";
    WordPrev          "word_prev"         ["b", "log:b", "diff:b"] File   "Previous word";
    Symbols           "symbols"           ["gO", "diff:gO"]        File   "Symbols in this file, to go to one";
    Definition        "go_to_definition"  ["gd", "diff:gd"]        File   "Go to the definition of the word under the cursor";
    RepoSymbols       "repo_symbols"      ["gS", "diff:gS"]        File   "Symbols in the repository, to go to one";
    References        "references"        ["gR", "diff:gR"]        File   "Where the word under the cursor is used";
    PrevColumn        "prev_column"       ["h"]                    Project "Previous column of the board";
    NextColumn        "next_column"       ["l"]                    Project "Next column of the board";
    MoveItem          "move_item"         [",c"]                   Project "Move the item to another column";
    SetField          "set_field"         [",f"]                   Page   "Set a field of the item in its project";
    AddItem           "add_item"          [",i"]                   Project "Add an issue, a pull request or a draft";
    ArchiveItem       "archive_item"      [",X"]                   Project "Archive the item, or restore it";

    NextHunk          "next_change"       ["]c", "}"]              Diff   "Next change";
    PrevHunk          "prev_change"       ["[c", "{"]              Diff   "Previous change";
    NextFile          "next_file"         ["]f", "]]", "<S-Down>"] Diff   "Next file";
    PrevFile          "prev_file"         ["[f", "[[", "<S-Up>"]   Diff   "Previous file";
    OldSide           "old_side"          ["<S-Left>"]             Diff   "Old side of a split row (for comments)";
    NewSide           "new_side"          ["<S-Right>"]            Diff   "New side of a split row (for comments)";
    NextUnviewed      "next_unviewed"     ["U"]                    Diff   "Next unviewed file";
    NextThread        "next_thread"       ["]t"]                   Diff   "Next unresolved thread";
    PrevThread        "prev_thread"       ["[t"]                   Diff   "Previous unresolved thread";
    JumpMove          "jump_move"         ["%"]                    Diff   "Jump to the other end of moved code";
    SwitchPane        "switch_pane"       ["<Tab>"]                Diff   "Switch between the file tree and the diff";
    Fold              "fold"              ["h"]                    Tree   "Fold the folder (on a file, go to its folder)";
    Unfold            "unfold"            ["l"]                    Tree   "Unfold the folder";
    TreeNarrower      "tree_narrower"     ["<"]                    Tree   "Narrow the file tree";
    TreeWider         "tree_wider"        [">"]                    Tree   "Widen the file tree";
    FoldClose         "fold_close"        ["zc", "page:zc"]        Diff   "Fold the file, thread or review";
    FoldOpen          "fold_open"         ["zo", "page:zo"]        Diff   "Unfold the file, thread or review";
    FoldToggle        "fold_toggle"       ["za", "page:za"]        Diff   "Fold or unfold the file, thread or review";
    FoldAll           "fold_all"          ["zM"]                   Diff   "Fold every file";
    UnfoldAll         "unfold_all"        ["zR"]                   Diff   "Unfold every file";
    ToggleTree        "toggle_tree"       ["zT"]                   Diff   "Show or hide the file tree";
    ToggleSplit       "toggle_split"      ["zs"]                   Diff   "Split or unified view";
    IgnoreWhitespace  "ignore_whitespace" ["zw"]                   Diff   "Ignore whitespace changes";
    FullFile          "full_file"         ["zf"]                   Diff   "Show the whole file";
    ExpandContext     "expand_context"    ["e"]                    Diff   "Show more context";
    ToggleSinceReview "since_review"      []                       Diff   "Show only changes since your last review";
    PickCommits       "pick_commits"      []                       Diff   "Choose commits to view";
    ToggleViewed      "toggle_viewed"     [",v"]                     Diff   "Mark the file viewed (syncs with GitHub)";
    MarkReviewed      "mark_reviewed"     ["m"]                    Diff   "Mark the change reviewed";
    VisualLines       "select_lines"      ["V", "x", "file:V"]     Diff   "Select lines";
    Suggest           "suggest"           []                       Diff   "Suggest a change (opens $EDITOR)";
    FileComment       "file_comment"      ["C"]                    Diff   "Comment on the whole file";
    ResolveThread     "resolve"           [",t"]                     Diff   "Resolve or unresolve the thread";
    SubmitReview      "submit_review"     [",s"]                   Diff   "Submit your review";
}

impl Action {
    /// `alt-1`…`alt-9`, in order.
    pub const OPEN_TABS: [Action; 9] = [
        Action::OpenTab1,
        Action::OpenTab2,
        Action::OpenTab3,
        Action::OpenTab4,
        Action::OpenTab5,
        Action::OpenTab6,
        Action::OpenTab7,
        Action::OpenTab8,
        Action::OpenTab9,
    ];

    /// The page's tabs 1…9, in order.
    pub const PAGE_TABS: [Action; 9] = [
        Action::Tab1,
        Action::Tab2,
        Action::Tab3,
        Action::Tab4,
        Action::Tab5,
        Action::Tab6,
        Action::Tab7,
        Action::Tab8,
        Action::Tab9,
    ];

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
                | HalfPageDown
                | HalfPageUp
                | PageDown
                | PageUp
                | ScrollDown
                | ScrollUp
                | ScrollCenter
                | ScrollTop
                | ScrollBottom
                | ScreenTop
                | ScreenMiddle
                | ScreenBottom
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
                | NextItem
                | PrevItem
        )
    }

    /// Motions a count repeats.
    pub fn repeats(self) -> bool {
        use Action::*;
        matches!(
            self,
            Down | Up
                | HalfPageDown
                | HalfPageUp
                | PageDown
                | PageUp
                | ScrollDown
                | ScrollUp
                | ScrollLeft
                | ScrollRight
                | Back
                | Forward
                | NextOpenTab
                | PrevOpenTab
                | SearchNext
                | SearchPrev
                | NextItem
                | PrevItem
                | NextSection
                | PrevSection
                | WordNext
                | WordPrev
                | NextHunk
                | PrevHunk
                | NextFile
                | PrevFile
                | NextThread
                | PrevThread
                | UpLevel
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

    /// A digit typed as (part of) a count.
    pub fn digit(self) -> Option<u32> {
        match self.code {
            KeyCode::Char(c) if self.mods.is_empty() => c.to_digit(10),
            _ => None,
        }
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

/// Parses a key sequence in vim notation: `gg`, `]c`, `<C-w>j`.
pub fn parse_keys(s: &str) -> Result<Vec<Key>, String> {
    let mut keys = Vec::new();
    let mut rest = s;
    while let Some(c) = rest.chars().next() {
        // `<…>` names a key; a lone `<` is itself.
        let len = match (c, rest.find('>')) {
            ('<', Some(end)) if end > 1 && !rest.get(1..end).is_some_and(|n| n.contains('<')) => {
                end + 1
            }
            ('<', None) if rest.len() > 1 => return Err(format!("unclosed `<` in `{s}`")),
            _ => c.len_utf8(),
        };
        let (key, tail) = rest.split_at(len);
        keys.push(
            match key.strip_prefix('<').and_then(|k| k.strip_suffix('>')) {
                Some(name) => parse_named(name).ok_or_else(|| format!("unknown key `{key}`"))?,
                None => Key::new(KeyCode::Char(c), KeyModifiers::NONE),
            },
        );
        rest = tail;
    }
    if keys.is_empty() {
        return Err("empty key".into());
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

/// Writes a key sequence in vim notation.
pub fn format_keys(keys: &[Key]) -> String {
    keys.iter().copied().map(format_key).collect()
}

/// What typed keys come to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolved {
    Run(Action),
    /// The start of longer bindings: wait for the next key.
    Wait,
    /// Nothing here.
    None,
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub keys: Vec<Key>,
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
            let keys: Vec<String> = match overrides.get(action.name()) {
                Some(list) => list.clone(),
                None => action.defaults().iter().map(|s| (*s).to_owned()).collect(),
            };
            for key in keys {
                let (key, context) = scoped(&key, action.context());
                let at = |e: String| format!("[keys] {}: {e}", action.name());
                let keys = parse_keys(key).map_err(at)?;
                if keys.first().and_then(|k| k.digit()).is_some() {
                    return Err(at(format!("`{key}` starts with a digit, which is a count")));
                }
                bindings.push(Binding {
                    keys,
                    action,
                    context,
                });
            }
        }
        let keymap = Self { bindings };
        keymap.validate()?;
        Ok(keymap)
    }

    /// Rejects two bindings that can be active at once when they're the
    /// same keys in one context, or one starts the other (it could never
    /// run).
    fn validate(&self) -> Result<(), String> {
        for (i, a) in self.bindings.iter().enumerate() {
            for b in self.bindings.iter().skip(i + 1) {
                let same = a.keys == b.keys && a.context == b.context;
                let starts = a.keys != b.keys
                    && (a.keys.starts_with(&b.keys) || b.keys.starts_with(&a.keys));
                if (same || starts) && a.context.meets(b.context) {
                    return Err(format!(
                        "key `{}` ({}) conflicts with `{}` ({})",
                        format_keys(&a.keys),
                        a.action.name(),
                        format_keys(&b.keys),
                        b.action.name()
                    ));
                }
            }
        }
        Ok(())
    }

    /// The bindings active in `stack`, innermost context first; one
    /// shadowed by an inner binding of the same keys is left out.
    fn active<'a>(&'a self, stack: &'a [Context]) -> impl Iterator<Item = &'a Binding> {
        let mut seen: Vec<&[Key]> = Vec::new();
        stack
            .iter()
            .flat_map(|c| self.bindings.iter().filter(move |b| b.context == *c))
            .filter(move |b| {
                let fresh = !seen.contains(&b.keys.as_slice());
                if fresh {
                    seen.push(&b.keys);
                }
                fresh
            })
    }

    /// What `keys`, typed so far, do in `stack`.
    pub fn resolve(&self, keys: &[Key], stack: &[Context]) -> Resolved {
        let mut longer = false;
        for b in self.active(stack) {
            if b.keys == keys {
                return Resolved::Run(b.action);
            }
            longer |= b.keys.starts_with(keys);
        }
        if longer {
            Resolved::Wait
        } else {
            Resolved::None
        }
    }

    /// What `keys` do in a context not active now, to say where they work.
    pub fn elsewhere(&self, keys: &[Key]) -> Option<Action> {
        self.bindings
            .iter()
            .find(|b| b.keys == keys)
            .map(|b| b.action)
    }

    /// The key sequences that run `action` in `stack`.
    pub fn keys_in<'a>(
        &'a self,
        action: Action,
        stack: &'a [Context],
    ) -> impl Iterator<Item = &'a [Key]> {
        self.active(stack)
            .filter(move |b| b.action == action)
            .map(|b| b.keys.as_slice())
    }

    /// What can follow `typed` in `stack`: the rest of each binding and
    /// what it does.
    pub fn continuations(&self, typed: &[Key], stack: &[Context]) -> Vec<(Vec<Key>, Action)> {
        self.active(stack)
            .filter(|b| b.keys.len() > typed.len() && b.keys.starts_with(typed))
            .map(|b| {
                (
                    b.keys.get(typed.len()..).unwrap_or_default().to_vec(),
                    b.action,
                )
            })
            .collect()
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

/// A key sequence as people write it: `gg`, `]c`, `ctrl-w j`.
pub fn pretty_keys(keys: &[Key]) -> String {
    let plain = keys
        .iter()
        .all(|k| k.mods.is_empty() && matches!(k.code, KeyCode::Char(c) if c != ' '));
    let shown: Vec<String> = keys.iter().copied().map(pretty).collect();
    shown.join(if plain { "" } else { " " })
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
                .map(|b| format!("`{}`", pretty_keys(&b.keys)))
                .collect()
        };
        let rows: Vec<(String, &str)> = Action::ALL
            .iter()
            .filter(|a| !Action::OPEN_TABS.get(1..).unwrap_or_default().contains(a))
            .filter_map(|&action| {
                let mut keys = keys_of(action);
                // Open tabs 1 to 9 are one row.
                if action == Action::OpenTab1 && !keys.is_empty() {
                    keys.extend(keys_of(Action::OpenTab9));
                    return Some((keys.join("–"), "Go to open tab 1 to 9"));
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

    fn keys(s: &str) -> Vec<Key> {
        parse_keys(s).unwrap()
    }

    fn parse_key(s: &str) -> Result<Key, String> {
        match parse_keys(s)?.as_slice() {
            [key] => Ok(*key),
            _ => Err(format!("`{s}` is more than one key")),
        }
    }

    const PAGE: &[Context] = &[Context::Page, Context::Global];
    const LIST: &[Context] = &[Context::List, Context::Page, Context::Global];
    const DIFF: &[Context] = &[Context::Diff, Context::Global];
    const TREE: &[Context] = &[Context::Tree, Context::Diff, Context::Global];

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
        assert_eq!(parse_key("<lt>").unwrap(), key('<'));
        assert_eq!(parse_key("<").unwrap(), key('<'));
        assert_eq!(parse_key("<C-i>").unwrap(), parse_key("<Tab>").unwrap());
        assert_eq!(keys("gg"), [key('g'), key('g')]);
        assert_eq!(
            keys("<C-w>j"),
            [
                Key::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
                key('j')
            ]
        );
        parse_key("<Nope>").unwrap_err();
        parse_key("<C-d").unwrap_err();
        parse_key("gg").unwrap_err();
        parse_keys("").unwrap_err();
    }

    #[test]
    fn formats_round_trip() {
        for s in [
            "G", "<C-d>", "<Enter>", "<Esc>", "<S-Tab>", "<lt>", "?", ":", "gg", "]c", "<C-w>j",
        ] {
            assert_eq!(format_keys(&keys(s)), s);
        }
    }

    #[test]
    fn shift_is_folded_into_chars() {
        let event = KeyEvent::new(KeyCode::Char('G'), KeyModifiers::SHIFT);
        assert_eq!(Key::from(event), key('G'));
        let event = KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT);
        assert_eq!(parse_key("<S-Tab>").unwrap(), Key::from(event));
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
    fn sequences_wait_then_run() {
        let keymap = Keymap::default();
        assert_eq!(keymap.resolve(&keys("g"), PAGE), Resolved::Wait);
        assert_eq!(
            keymap.resolve(&keys("gg"), PAGE),
            Resolved::Run(Action::Top)
        );
        assert_eq!(keymap.resolve(&keys("gz"), PAGE), Resolved::None);
        assert_eq!(keymap.resolve(&keys("z"), PAGE), Resolved::Wait);
        assert_eq!(
            keymap.resolve(&keys("]c"), DIFF),
            Resolved::Run(Action::NextHunk)
        );
        assert_eq!(keymap.resolve(&keys("]c"), PAGE), Resolved::None);
        assert_eq!(keymap.elsewhere(&keys("]c")), Some(Action::NextHunk));
        let next: Vec<_> = keymap.continuations(&keys("g"), PAGE);
        assert!(
            next.iter()
                .any(|(k, a)| *k == keys("t") && *a == Action::NextTab)
        );
        assert!(
            !next.iter().any(|(_, a)| *a == Action::Blame),
            "gb is the file's"
        );
    }

    #[test]
    fn inner_contexts_win() {
        let keymap = Keymap::default();
        assert_eq!(
            keymap.resolve(&keys("t"), LIST),
            Resolved::Run(Action::ToggleState)
        );
        assert_eq!(
            keymap.resolve(&keys("zT"), DIFF),
            Resolved::Run(Action::ToggleTree)
        );
        // Writes to GitHub wait behind `,`; H M L are vim's screen motions.
        assert_eq!(keymap.resolve(&keys(","), PAGE), Resolved::Wait);
        assert_eq!(
            keymap.resolve(&keys(",m"), PAGE),
            Resolved::Run(Action::Merge)
        );
        assert_eq!(
            keymap.resolve(&keys("M"), PAGE),
            Resolved::Run(Action::ScreenMiddle)
        );
        assert_eq!(
            keymap.resolve(&keys("]]"), DIFF),
            Resolved::Run(Action::NextFile)
        );
        assert_eq!(
            keymap.resolve(&keys("]]"), TREE),
            Resolved::Run(Action::NextSection)
        );
        assert_eq!(
            keymap.resolve(&keys("h"), DIFF),
            Resolved::Run(Action::ScrollLeft)
        );
        assert_eq!(
            keymap.resolve(&keys("h"), TREE),
            Resolved::Run(Action::Fold)
        );
        assert_eq!(
            keymap.resolve(&keys("<Tab>"), PAGE),
            Resolved::Run(Action::Forward)
        );
        assert_eq!(
            keymap.resolve(&keys("<Tab>"), DIFF),
            Resolved::Run(Action::SwitchPane)
        );
        assert_eq!(
            keymap.resolve(&keys("<Esc>"), PAGE),
            Resolved::Run(Action::Cancel)
        );
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
        reject("down", "k", "conflicts");
        // `g` alone would make `gg` unreachable.
        reject("star", "g", "conflicts");
        reject("star", "3", "count");
        reject("star", "<Nope>", "unknown key");
        reject("star", "<C-d", "unclosed");
        // An inner context may shadow a global key, as the diff's `⇥`
        // does `ctrl-i`; it can't start a longer one, though.
        let shadow = HashMap::from([("star".to_owned(), vec!["page:q".to_owned()])]);
        let keymap = Keymap::with_overrides(&shadow).unwrap();
        assert_eq!(
            keymap.resolve(&keys("q"), PAGE),
            Resolved::Run(Action::Star)
        );
        assert_eq!(
            keymap.resolve(&keys("q"), DIFF),
            Resolved::Run(Action::CloseView)
        );
        let scoped = HashMap::from([("star".to_owned(), vec!["page:z".to_owned()])]);
        assert!(
            Keymap::with_overrides(&scoped)
                .unwrap_err()
                .contains("conflicts")
        );
    }

    #[test]
    fn overrides_replace_defaults() {
        let overrides = HashMap::from([("down".to_owned(), vec!["<C-n>".to_owned()])]);
        let keymap = Keymap::with_overrides(&overrides).unwrap();
        let ctrl_n = keys("<C-n>");
        assert_eq!(keymap.resolve(&ctrl_n, PAGE), Resolved::Run(Action::Down));
        assert_eq!(keymap.resolve(&keys("j"), PAGE), Resolved::None);
        assert!(keymap.keys_in(Action::Down, PAGE).eq([ctrl_n.as_slice()]));
    }

    #[test]
    fn keys_read_like_people_write_them() {
        assert_eq!(pretty(parse_key("<Enter>").unwrap()), "↵");
        assert_eq!(pretty_keys(&keys("<C-d>")), "ctrl-d");
        assert_eq!(pretty_keys(&keys("gg")), "gg");
        assert_eq!(pretty_keys(&keys("]c")), "]c");
        assert_eq!(pretty_keys(&keys("<C-w>j")), "ctrl-w j");
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
            /// Writing keys down and reading them back gives the same keys.
            #[test]
            fn keys_round_trip(keys in prop::collection::vec(key(), 1..4)) {
                let written = format_keys(&keys);
                prop_assert_eq!(parse_keys(&written), Ok(keys), "{}", written);
            }

            /// Any accepted spelling settles on one canonical form.
            #[test]
            fn parsing_settles(s in "(<[A-Za-z-]{1,8}>|[ -~]){1,3}") {
                if let Ok(keys) = parse_keys(&s) {
                    let canonical = format_keys(&keys);
                    let again = parse_keys(&canonical).map(|k| format_keys(&k));
                    prop_assert_eq!(again, Ok(canonical));
                }
            }
        }
    }
}
