//! The feature table: which feature does an action never depends on the
//! table's order.

use super::*;
use crate::features::FEATURES;

/// No two features own one action.
#[test]
fn no_action_has_two_owners() {
    for &action in Action::ALL {
        let owners: Vec<usize> = (FEATURES.iter().enumerate())
            .filter(|(_, f)| f.owns.contains(&action))
            .map(|(i, _)| i)
            .collect();
        assert!(
            owners.len() < 2,
            "{action:?} is owned by features {owners:?}"
        );
    }
}

/// On every screen the fixtures reach, no two features take one action.
#[test]
fn no_action_is_claimed_twice() {
    let keys = super::keys::screens().map(|(name, make)| (name, make()));
    let screens = super::screens::pages().into_iter().chain(keys);
    let mut twice = Vec::new();
    for (name, s) in screens.chain(super::smoke::more()) {
        for &action in Action::ALL {
            let takers: Vec<usize> = (FEATURES.iter().enumerate())
                .filter(|(_, f)| (f.takes)(&s, action))
                .map(|(i, _)| i)
                .collect();
            if takers.len() > 1 {
                twice.push(format!("{name}: {action:?} by features {takers:?}"));
            }
        }
    }
    assert!(twice.is_empty(), "{}", twice.join("\n"));
}
