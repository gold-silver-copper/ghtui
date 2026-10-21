//! For the fuzz targets, not the app: any JSON decoded as one of GitHub's
//! wire shapes and made into ghtui's model of it, as the client does.
//! Decoding may fail; making the model of what decoded must not panic.

use serde::de::DeserializeOwned;

use crate::api::browse::{self as b, wire};
use crate::api::model::{self, RepoId};
use crate::api::queries as q;

fn decoded<T: DeserializeOwned>(json: &[u8]) -> Option<T> {
    serde_json::from_slice(json).ok()
}

/// How many shapes [`decode`] picks among.
pub const SHAPES: u8 = 33;

/// Decodes `json` as the shape `shape` picks and makes its model.
pub fn decode(shape: u8, json: &[u8]) {
    let repo = RepoId::new("o", "r");
    macro_rules! model {
        ($ty:ty, $make:expr) => {
            if let Some(v) = decoded::<$ty>(json) {
                let _ = $make(v);
            }
        };
    }
    match shape % SHAPES {
        0 => model!(q::RepositoryWithPr, model::PrDetail::from_wire),
        1 => model!(q::ReviewThread, model::ReviewThread::from_wire),
        2 => model!(b::ChecksCommit, |v: b::ChecksCommit| {
            let first = b::rollup_contexts(v.status_check_rollup);
            b::checks(v.oid.0, model::Capped::new(first.items, first.total))
        }),
        3 => model!(b::wire_branches::Branch, |v: b::wire_branches::Branch| v
            .into_info(&repo, Some("main"))),
        4 => model!(
            b::wire_milestones::Milestone,
            b::wire_milestones::Milestone::into_info
        ),
        5 => model!(
            b::wire_deployments::Deployment,
            b::wire_deployments::Deployment::into_info
        ),
        6 => model!(b::rest_compare::Compare, |v: b::rest_compare::Compare| v
            .into_comparison(false)),
        7 => model!(b::rest_compare::Compare, |v: b::rest_compare::Compare| v
            .into_comparison(true)),
        8 => model!(b::wire_blame::Blame, b::wire_blame::Blame::into_blame),
        9 => model!(b::rest_gists::Gist, b::rest_gists::Gist::into_gist),
        10 => model!(b::wire_gists::Gist, b::wire_gists::Gist::into_summary),
        11 => model!(b::wire_teams::Detail, b::wire_teams::Detail::into_detail),
        12 => model!(
            b::rest_advisories::Advisory,
            b::rest_advisories::Advisory::into_advisory
        ),
        13 => model!(b::rest_search::Commit, b::rest_search::Commit::into_hit),
        14 => model!(b::rest_search::Code, b::rest_search::Code::into_hit),
        15 => model!(b::wire_discussions::Hit, b::wire_discussions::Hit::into_hit),
        16 => model!(
            b::wire_discussions::Detail,
            |v: b::wire_discussions::Detail| v.into_detail(repo.clone())
        ),
        17 => model!(
            b::WireContributions,
            b::WireContributions::into_contributions
        ),
        18 => model!(b::PagedRepos, |v: b::PagedRepos| v
            .filter_results(b::RepoCard::into_summary)),
        19 => model!(b::PagedStars, |v: b::PagedStars| v
            .filter_results(b::RepoCard::into_summary)),
        20 => model!(b::ReleaseList, |v: b::ReleaseList| v
            .filter_results(|r| Some(r.into_release()))),
        21 => model!(b::WireReleaseFull, b::WireReleaseFull::into_release),
        22 => model!(b::TagRefs, |v: b::TagRefs| v
            .filter_results(b::TagRef::into_info)),
        23 => model!(b::WireCommit, b::WireCommit::into_detail),
        24 => model!(b::RepoFull, |v: b::RepoFull| v.into_overview()),
        25 => model!(b::BrowseItem, b::BrowseItem::into_issue),
        26 => model!(b::BrowseItem, b::BrowseItem::into_user),
        27 => model!(b::IssueFull, b::IssueFull::into_detail),
        28 => model!(b::WirePrActivity, b::WirePrActivity::into_activity),
        29 => model!(b::ProfileQuery, b::ProfileQuery::into_profile),
        30 => model!(
            (b::rest_actions::Job, b::rest_actions::Run),
            |(job, run): (b::rest_actions::Job, b::rest_actions::Run)| job
                .into_job(run.into_job_run(None))
        ),
        31 => model!(b::CommitHistory, |v: b::CommitHistory| v
            .filter_results(|c| Some(c.into_info()))),
        _ => model!(
            wire::Connection<b::wire_branches::Branch>,
            |v: wire::Connection<b::wire_branches::Branch>| v
                .into_results(|b| b.into_info(&repo, None))
        ),
    }
}
