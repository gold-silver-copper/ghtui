//! Merging an open pull request: now, once every requirement is met
//! (auto-merge), or through its base branch's merge queue; and taking
//! back auto-merge or a place in the queue.

use crate::api::change::{Change, MergeMethod};
use crate::api::model::{ChecksState, PrDetail, PrRef, Readiness};
use crate::ui::pages::{auto_merging, queue_place};

use super::{Plan, merge_facts};
use crate::keymap::Action;
use crate::state::State;

/// What merging does: once it's queued or set to merge, it's taken back.
pub(super) fn plan(state: &State, pr: &PrRef, d: &PrDetail) -> Result<Plan, String> {
    let key = |a: Action| state.first_key(a);
    if let Some(queued) = &d.queued {
        if !d.may.merge {
            return Err(format!(
                "Removing it from the merge queue takes write access to {}",
                pr.repo
            ));
        }
        let facts = vec![(queue_place(queued, &d.base_ref), false)];
        let change = Change::Dequeue { pr: d.id.clone() };
        let title = format!("Remove {pr} from the merge queue?");
        return Ok(Plan::Ask(
            title,
            facts,
            vec![("Remove from the queue".into(), change)],
        ));
    }
    if let Some(auto) = &d.auto_merge {
        if !d.may.disable_auto_merge {
            return Err(format!(
                "{} turned on auto-merge, and GitHub doesn't let you turn it off",
                auto.by
            ));
        }
        let facts = vec![(auto_merging(auto, d.merge_queue), false)];
        let change = Change::DisableAutoMerge { pr: d.id.clone() };
        let title = format!("Disable auto-merge for {pr}?");
        return Ok(Plan::Ask(
            title,
            facts,
            vec![("Disable auto-merge".into(), change)],
        ));
    }
    let readiness = d.readiness();
    match readiness {
        _ if !d.may.merge => return Err(format!("Merging takes write access to {}", pr.repo)),
        _ if d.merge_methods.is_empty() => {
            return Err("The repository allows no way of merging".into());
        }
        Readiness::Conflicts => {
            return Err(format!(
                "It conflicts with {}: resolve that on GitHub",
                d.base_ref
            ));
        }
        Readiness::Behind => {
            return Err(format!(
                "{} requires its branch up to date with {}: {} updates it",
                pr.repo,
                d.base_ref,
                key(Action::UpdateBranch)
            ));
        }
        _ => {}
    }
    // A queue merges as it's configured to, past any administrator.
    let now = readiness != Readiness::Blocked || (d.may.merge_as_admin && !d.merge_queue);
    let mut choices = Vec::new();
    let mut later = Vec::new();
    if now && d.merge_queue {
        let change = Change::Enqueue {
            pr: d.id.clone(),
            head: d.head_oid.clone(),
        };
        choices.push(("Add to the merge queue".to_owned(), change));
    } else if now {
        choices.extend(d.merge_methods.iter().map(|&method| {
            let change = Change::Merge {
                pr: d.id.clone(),
                method,
                head: d.head_oid.clone(),
            };
            (method.describe().to_owned(), change)
        }));
    }
    if d.may.enable_auto_merge {
        let when_ready = |method: MergeMethod| Change::EnableAutoMerge {
            pr: d.id.clone(),
            method,
            head: d.head_oid.clone(),
        };
        // The queue's own settings say how it merges.
        if d.merge_queue {
            if let Some(&method) = d.merge_methods.first() {
                later.push(("Merge when ready".to_owned(), when_ready(method)));
            }
        } else {
            later.extend(d.merge_methods.iter().map(|&method| {
                let label = format!("{} when ready", method.describe());
                (label, when_ready(method))
            }));
        }
    }
    if choices.is_empty() && later.is_empty() {
        let why = "GitHub says it's blocked: a required review or check is missing";
        return Err(if d.auto_merge_allowed {
            why.to_owned()
        } else {
            format!("{why}, and {} doesn't allow auto-merge", pr.repo)
        });
    }
    let mut facts = merge_facts(state, d);
    if d.merge_queue {
        facts.push((
            format!("{} merges through a merge queue", d.base_ref),
            false,
        ));
    }
    if !later.is_empty() {
        facts.push((
            "“When ready” merges it once every required review and check is in".into(),
            false,
        ));
    }
    // While checks run, waiting for them comes first.
    if d.checks == Some(ChecksState::Pending) {
        later.append(&mut choices);
        choices = later;
    } else {
        choices.append(&mut later);
    }
    Ok(Plan::Ask(
        format!("Merge {pr}: {}?", d.title),
        facts,
        choices,
    ))
}

/// The menu's name for merging it, as it is now.
pub(super) fn label(d: &PrDetail) -> &'static str {
    if d.queued.is_some() {
        "Remove from the merge queue"
    } else if d.auto_merge.is_some() {
        "Disable auto-merge"
    } else if d.merge_queue {
        "Add to the merge queue"
    } else {
        Action::Merge.description()
    }
}
