//! Where each piece of data comes from: git where it can answer
//! ([`crate::source`]), else GitHub's API; and the cache, which keeps what
//! the API answered.

use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use super::{Data, DataKey};
use crate::api::model::PrRef;
use crate::api::{ApiError, GitHub};
use crate::diff_job::GitContext;
use crate::source::{From, Local};
use crate::state::Msg;
use crate::store::Cached;

/// `key`'s data (for a list, its page after `after`), from git where it
/// can, else GitHub's API; and, when git could have answered, where it
/// came from. With `early`, what shows before the rest arrives is sent
/// there.
pub async fn load(
    gh: &GitHub,
    local: &Arc<Local>,
    key: &DataKey,
    after: Option<String>,
    early: Option<&UnboundedSender<Msg>>,
) -> (Result<Data, ApiError>, Option<From>) {
    if crate::source::by_git(key, after.as_deref()) {
        let (result, from) = local.fetch(gh, key, after).await;
        return (result, Some(from));
    }
    (fetch(gh, local.git(), key, after, early).await, None)
}

/// What the cache has for `key`, and when it was fetched.
pub fn cached(gh: &GitHub, key: &DataKey) -> Option<Cached<Data>> {
    if let Some(found) = key.cache_id().and_then(|id| gh.store().query_get(&id)) {
        return Some(found);
    }
    // A pull request it is, as kept: offline, its page opens.
    let DataKey::Issue(repo, number) = key else {
        return None;
    };
    let pr = PrRef {
        repo: repo.clone(),
        number: *number,
    };
    Some(Cached {
        value: Data::Issue(None),
        fetched_at: gh.cached_pull_request(&pr)?.fetched_at,
    })
}

/// `key`'s data from GitHub's API (git reads a thread's lines, and a
/// wiki); for a list, its page after `after`. A first page is kept in
/// the cache, if `key`'s data is ([`DataKey::kept`]). With `early`, a
/// search's page is sent there as soon as it's found, as a first look
/// while its pull requests' checks are read.
pub async fn fetch(
    gh: &GitHub,
    git: &GitContext,
    key: &DataKey,
    after: Option<String>,
    early: Option<&UnboundedSender<Msg>>,
) -> Result<Data, ApiError> {
    let first = after.is_none();
    let data = answer(gh, git, key, after, early).await?;
    if first && let Some(id) = key.cache_id() {
        gh.store().query_put(&id, &data);
    }
    Ok(data)
}

async fn answer(
    gh: &GitHub,
    git: &GitContext,
    key: &DataKey,
    after: Option<String>,
    early: Option<&UnboundedSender<Msg>>,
) -> Result<Data, ApiError> {
    use DataKey as K;
    Ok(match key {
        K::Repo(repo) => Data::Repo(Box::new(gh.repo(repo).await?)),
        K::Readme(repo) => Data::Readme(gh.readme(repo).await?.map(Box::new)),
        K::Tree(repo, rev, path) => Data::Tree(gh.tree(repo, rev, path).await?),
        K::Blob(repo, rev, path) => Data::Blob(Box::new(gh.blob(repo, rev, path).await?)),
        K::Search(kind, query) => {
            let found = |r: &crate::api::browse::SearchResults| {
                if let Some(tx) = early {
                    let _ = tx.send(Msg::Fetched {
                        key: key.clone(),
                        result: Ok(Data::Search(Box::new(r.clone()))),
                        cached_at: Some(crate::store::now()),
                    });
                }
            };
            let results = gh.search_showing(*kind, query, after, found).await?;
            Data::Search(Box::new(results))
        }
        K::StateCounts(query) => Data::StateCounts(gh.state_counts(query).await?),
        K::Issue(repo, number) => Data::Issue(gh.issue(repo, *number).await?.map(Box::new)),
        K::PrActivity(pr) => Data::PrActivity(Box::new(gh.pr_activity(pr).await?)),
        K::Reviews(pr) => Data::Reviews(Box::new(gh.reviews(pr).await?)),
        K::InProjects(repo, n) => Data::InProjects(Box::new(gh.projects_of(repo, *n).await?)),
        K::Lines(repo, commit, path) => {
            let lines = crate::talk::read_lines(git, repo, commit, path).await?;
            Data::Lines(Arc::new(lines))
        }
        K::CommitComments(repo, oid) => {
            Data::CommitComments(Box::new(gh.commit_comments(repo, oid).await?))
        }
        K::Profile(login) => Data::Profile(Box::new(gh.profile(login).await?)),
        K::Files(repo, rev) => {
            let (files, truncated) = gh.file_list(repo, rev).await?;
            Data::Files(Arc::new(files), truncated)
        }
        K::Refs(repo) => Data::Refs(Box::new(gh.refs(repo).await?)),
        K::RefIn(repo, spot) => Data::RefIn(gh.ref_in(repo, spot).await?),
        K::LastCommits(repo, rev, path) => {
            let names: Vec<String> = (gh.tree(repo, rev, path).await?.into_iter())
                .map(|e| e.name)
                .collect();
            let found = gh.last_commits(repo, rev, path, &names).await?;
            Data::LastCommits(Arc::new(found))
        }
        K::Commit(repo, oid) => Data::Commit(Box::new(gh.commit(repo, oid).await?)),
        K::History(repo, rev, path) => {
            Data::History(Box::new(gh.history(repo, rev, path, after).await?))
        }
        K::PrChecks(pr) => Data::Checks(Box::new(gh.pr_checks(pr).await?)),
        K::Users(list) => Data::Users(Box::new(gh.users(list, after).await?)),
        K::Forks(repo) => Data::RepoPage(Box::new(gh.forks(repo, after).await?)),
        K::Releases(repo) => Data::Releases(Box::new(gh.releases(repo, after).await?)),
        K::Release(repo, tag) => Data::Release(Box::new(gh.release(repo, tag).await?)),
        K::Tags(repo) => Data::Tags(Box::new(gh.tags(repo, after).await?)),
        K::Branches(repo) => Data::Branches(Box::new(gh.branches(repo, after).await?)),
        K::Wiki(repo, page) => Data::Wiki(Box::new(
            crate::wiki::page(git, repo, page.as_deref()).await?,
        )),
        K::Advisories(repo) => {
            Data::Advisories(Box::new(gh.advisories(repo.as_ref(), after).await?))
        }
        K::Advisory(repo, ghsa) => {
            Data::Advisory(Box::new(gh.advisory(repo.as_ref(), ghsa).await?))
        }
        K::Teams(org) => Data::Teams(Box::new(gh.teams(org, after).await?)),
        K::Team(org, slug) => Data::Team(Box::new(gh.team(org, slug).await?)),
        K::Gist(id) => Data::Gist(Box::new(gh.gist(id).await?)),
        K::Gists(login) => Data::Gists(Box::new(gh.gists(login, after).await?)),
        K::Blame(repo, rev, path) => Data::Blame(Box::new(gh.blame(repo, rev, path).await?)),
        K::Compare(repo, spec) => Data::Compare(Box::new(gh.compare(repo, spec).await?)),
        K::Deployments(repo, env) => {
            Data::Deployments(Box::new(gh.deployments(repo, env.as_deref(), after).await?))
        }
        K::Milestones(repo, closed) => {
            Data::Milestones(Box::new(gh.milestones(repo, *closed, after).await?))
        }
        K::Milestone(repo, n) => Data::Milestone(Box::new(gh.milestone(repo, *n, after).await?)),
        K::Discussions(of, category) => Data::Discussions(Box::new(
            gh.discussions(of, category.as_deref(), after).await?,
        )),
        K::Discussion(of, n) => Data::Discussion(Box::new(gh.discussion(of, *n).await?)),
        K::Run(repo, run, attempt) => {
            Data::Run(Box::new(gh.workflow_run(repo, *run, *attempt).await?))
        }
        K::Job(repo, job) => Data::Job(Box::new(gh.job(repo, *job).await?)),
        K::JobLog(repo, job) => Data::Log(Arc::new(gh.job_log(repo, *job).await?)),
        K::Workflow(repo, file) => Data::Workflow(Box::new(gh.workflow(repo, file).await?)),
        K::Runs(repo, query) => Data::Runs(Box::new(gh.runs(repo, query, after).await?)),
        K::CommitChecks(repo, oid) => Data::Checks(Box::new(gh.commit_checks(repo, oid).await?)),
        K::OwnerRepos(login, sort) => {
            Data::RepoPage(Box::new(gh.owner_repos(login, *sort, after).await?))
        }
        K::Stars(login) => Data::RepoPage(Box::new(gh.starred(login, after).await?)),
        K::Notifications(query) => {
            Data::Notifications(Box::new(gh.notifications(query, after).await?))
        }
        K::Sidebar(repo, number) => Data::Sidebar(Box::new(gh.sidebar(repo, *number).await?)),
        K::NewForm(repo) => Data::NewForm(Box::new(gh.new_form(repo).await?)),
        K::Projects(of) => Data::Projects(Box::new(gh.projects(of, after).await?)),
        K::Project(p, view) => Data::Project(Box::new(gh.project(p, *view, after).await?)),
        K::Alerts(repo, kind, query) => {
            Data::Alerts(Box::new(gh.alerts(repo, *kind, query, after).await?))
        }
        K::Alert(a) => Data::Alert(Box::new(gh.alert(a).await?)),
        K::Subscription(of) => Data::Subscription(Box::new(gh.subscription(of).await?)),
        K::Settings(repo, page) => Data::Settings(Box::new(gh.settings(repo, *page).await?)),
        K::Activity(repo) => Data::Activity(Box::new(gh.activity(repo).await?)),
        K::Traffic(repo) => Data::Traffic(Box::new(gh.traffic(repo).await?)),
        K::Dependencies(repo) => Data::Dependencies(Box::new(gh.dependencies(repo).await?)),
        K::Featured => Data::Topics(Box::new(gh.featured_topics().await?)),
        K::Topic(name) => Data::Topic(Box::new(gh.topic(name).await?)),
        K::StarLists(login) => Data::StarLists(Box::new(gh.star_lists(login).await?)),
        K::StarList(login, slug) => {
            Data::StarList(Box::new(gh.star_list(login, slug, after).await?))
        }
        K::MyLists(repo) => Data::MyLists(Box::new(gh.my_lists(repo).await?)),
        K::Packages(of) => Data::Packages(Box::new(gh.packages(of).await?)),
        K::Package(p) => Data::Package(Box::new(gh.package(p, after).await?)),
        K::SavedReplies => Data::SavedReplies(Box::new(gh.saved_replies().await?)),
        K::Labels(repo) => Data::Labels(Box::new(gh.labels(repo, after).await?)),
        K::Owners => Data::Owners(Box::new(gh.owners().await?)),
        K::Templates => Data::Templates(Box::new(gh.templates().await?)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::model::RepoId;
    use crate::store::Store;

    fn github(dir: &tempfile::TempDir) -> GitHub {
        let store = Store::open(&dir.path().join("cache.redb"));
        GitHub::new(
            crate::api::auth::Token::new("t"),
            store,
            crate::host::Host::GITHUB_COM,
        )
    }

    /// What's kept is read back by its key alone; what isn't kept, isn't.
    #[test]
    fn kept_data_is_read_back_by_its_key() {
        let dir = tempfile::tempdir().unwrap();
        let gh = github(&dir);
        let repo = RepoId::new("o", "r");
        let readme = DataKey::Readme(repo.clone());
        assert_eq!(cached(&gh, &readme), None);
        let data = Data::Readme(None);
        gh.store().query_put(&readme.cache_id().unwrap(), &data);
        assert_eq!(cached(&gh, &readme).map(|c| c.value), Some(data));
        assert_eq!(cached(&gh, &DataKey::Readme(RepoId::new("o", "s"))), None);
        let form = DataKey::NewForm(repo);
        assert_eq!(form.cache_id(), None, "what a form offers is asked anew");
    }
}
