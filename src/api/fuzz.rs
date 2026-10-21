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
    // Decodes `json` as `$ty` and, if it does, makes its model from `$v`.
    macro_rules! model {
        ($ty:ty, $v:pat => $make:expr) => {
            if let Some($v) = decoded::<$ty>(json) {
                let _ = $make;
            }
        };
    }
    match shape % SHAPES {
        0 => model!(q::RepositoryWithPr, v => model::PrDetail::from_wire(v)),
        1 => model!(q::ReviewThread, v => model::ReviewThread::from_wire(v)),
        2 => model!(b::ChecksCommit, v => {
            let first = b::rollup_contexts(v.status_check_rollup);
            b::checks(v.oid.0, model::Capped::new(first.items, first.total))
        }),
        3 => model!(b::wire_branches::Branch, v => v.into_info(&repo, Some("main"))),
        4 => model!(b::wire_milestones::Milestone, v => v.into_info()),
        5 => model!(b::wire_deployments::Deployment, v => v.into_info()),
        6 => model!(b::rest_compare::Compare, v => v.into_comparison(false)),
        7 => model!(b::rest_compare::Compare, v => v.into_comparison(true)),
        8 => model!(b::wire_blame::Blame, v => v.into_blame()),
        9 => model!(b::rest_gists::Gist, v => v.into_gist()),
        10 => model!(b::wire_gists::Gist, v => v.into_summary()),
        11 => model!(b::wire_teams::Detail, v => v.into_detail()),
        12 => model!(b::rest_advisories::Advisory, v => v.into_advisory()),
        13 => model!(b::rest_search::Commit, v => v.into_hit()),
        14 => model!(b::rest_search::Code, v => v.into_hit()),
        15 => model!(b::wire_discussions::Hit, v => v.into_hit()),
        16 => model!(b::wire_discussions::Detail, v => v.into_detail(repo)),
        17 => model!(b::WireContributions, v => v.into_contributions()),
        18 => model!(b::PagedRepos, v => v.filter_results(b::RepoCard::into_summary)),
        19 => model!(b::PagedStars, v => v.filter_results(b::RepoCard::into_summary)),
        20 => model!(b::ReleaseList, v => v.filter_results(|r| Some(r.into_release()))),
        21 => model!(b::WireReleaseFull, v => v.into_release()),
        22 => model!(b::TagRefs, v => v.filter_results(b::TagRef::into_info)),
        23 => model!(b::WireCommit, v => v.into_detail()),
        24 => model!(b::RepoFull, v => v.into_overview()),
        25 => model!(b::BrowseItem, v => v.into_issue()),
        26 => model!(b::BrowseItem, v => v.into_user()),
        27 => model!(b::IssueFull, v => v.into_detail()),
        28 => model!(b::WirePrActivity, v => v.into_activity()),
        29 => model!(b::ProfileQuery, v => v.into_profile()),
        30 => model!(
            (b::rest_actions::Job, b::rest_actions::Run),
            (job, run) => job.into_job(run.into_job_run(None))
        ),
        31 => model!(b::CommitHistory, v => v.filter_results(|c| Some(c.into_info()))),
        _ => model!(
            wire::Connection<b::wire_branches::Branch>,
            v => v.into_results(|b| b.into_info(&repo, None))
        ),
    }
}
