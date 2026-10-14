//! Moving around pages, the way you'd use the website: selecting rows,
//! scrolling, following links (by keys, letters or the mouse), tabs,
//! history, the search box with its suggestions, go to file, branches, the
//! actions menu, and the keys hinted in the status bar.

use std::mem;
use std::sync::Arc;

use crate::api::browse::{RepoSummary, SearchKind};
use crate::api::change::Change;
use crate::api::model::RepoId;
use crate::diff::anchor::Side;
use crate::theme::{Bg, Theme};
use crate::ui::chrome::{self, KeyRow, SuggestRow};
use crate::ui::diff_doc::Row;
use crate::ui::page::{self, HintLabel, Item, Link, Page};
use crate::ui::pages::{self, PrTab, ProfileTab};
use crate::ui::text::{self, Word};
use crate::ui::{PAD_X, PAD_Y};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui_textarea::TextArea;
use serde::{Deserialize, Serialize};

use crate::browse;
use crate::data::{Data, DataKey};
use crate::diff_screen::{self, Pane};
use crate::keymap::Action;
use crate::picker::{fuzzy_score, move_in_list};
use crate::review::ComposeTarget;
use crate::route::{self, Dest, Route, Target};
use crate::state::{Api, Cmd, Overlay, Remote, Screen, State, apply};

/// Rows `j`/`k` scroll when there's no row to move to nearby.
const STEP: isize = 3;
/// Rows kept around the selection when scrolling to it.
const MARGIN: usize = 2;
/// Recently visited pages kept for the search box.
const VISITS: usize = 300;

mod copy;
mod doables;
mod find;
mod hints;
mod history;
mod keys;
mod lists;
mod menu;
mod mouse;
mod screen;
mod search;
mod write;

pub use copy::*;
pub use doables::*;
pub use find::*;
pub use hints::*;
pub use history::*;
pub use keys::*;
use lists::*;
pub use menu::*;
pub use mouse::*;
pub use screen::*;
pub use search::*;
pub use write::*;
