//! Marking everything the inbox shows read, or done.

use crate::api::change::Change;
use crate::api::notifications::{Inbox, InboxFilter};

use super::Plan;
use crate::keymap::Action;

/// Marking what the inbox `query` lists shows read or done: asked first,
/// as it changes many threads, saying how many. REST marks read
/// everything up to a time, all of the inbox or a repository's; a filter
/// it can't apply that way (a reason) is marked thread by thread, as
/// done always is. `loaded` is when the inbox was fetched.
pub(super) fn plan(
    query: &str,
    inbox: &Inbox,
    loaded: Option<u64>,
    action: Action,
) -> Result<Plan, String> {
    let filter = InboxFilter::parse(query);
    let rows = &inbox.list.items;
    let more = inbox.list.next.is_some();
    let threads = |n: usize| match n {
        1 => "1 notification".to_owned(),
        n => format!("{n} notifications"),
    };
    let mut facts = Vec::new();
    let (title, label, change) = if action == Action::MarkAllRead {
        let unread: Vec<String> = (rows.iter().filter(|n| n.unread))
            .map(|n| n.id.clone())
            .collect();
        if unread.is_empty() {
            return Err("Nothing shown is unread".into());
        }
        let title = format!("Mark {} read?", threads(unread.len()));
        let whole = filter.reasons.is_empty() && !filter.participating;
        // Up to when the inbox was fetched: a thread updated after isn't
        // one you saw, and stays unread. Not up to the newest shown: GitHub
        // keeps a thread's time a little past the one it says, and leaves
        // a thread marked read up to the time it says unread.
        let newest = rows.iter().map(|n| n.updated_at.clone()).max();
        let upto = loaded.map(crate::ui::time::iso8601).or(newest);
        let change = match upto {
            Some(upto) if whole => {
                if more {
                    let older = "And the older ones further down, not loaded yet";
                    facts.push((older.to_owned(), false));
                }
                Change::MarkAllRead {
                    repo: filter.repo,
                    last_read_at: upto,
                }
            }
            _ => {
                if more {
                    facts.push((loaded_only(rows.len()), false));
                }
                Change::MarkEach {
                    threads: unread,
                    done: false,
                }
            }
        };
        (title, "Mark read", change)
    } else {
        if rows.is_empty() {
            return Err("Nothing is shown".into());
        }
        facts.push(("Done takes them out of your inbox".to_owned(), false));
        if more {
            facts.push((loaded_only(rows.len()), false));
        }
        let change = Change::MarkEach {
            threads: rows.iter().map(|n| n.id.clone()).collect(),
            done: true,
        };
        (
            format!("Mark {} done?", threads(rows.len())),
            "Mark done",
            change,
        )
    };
    Ok(Plan::Ask(title, facts, vec![(label.to_owned(), change)]))
}

/// "Only the 30 loaded: those further down stay as they are".
fn loaded_only(n: usize) -> String {
    format!("Only the {n} loaded: those further down stay as they are")
}

/// Whether the inbox shows a change to many threads: none it was about
/// is unread. Done shows as read: GitHub's API goes on listing a thread
/// marked done.
pub(super) fn shows(inbox: &Inbox, change: &Change) -> bool {
    let rows = &inbox.list.items;
    match change {
        Change::MarkAllRead { last_read_at, .. } => rows
            .iter()
            .all(|n| !n.unread || n.updated_at.as_str() > last_read_at.as_str()),
        Change::MarkEach { threads, .. } => rows
            .iter()
            .filter(|n| threads.contains(&n.id))
            .all(|n| !n.unread),
        _ => true,
    }
}
