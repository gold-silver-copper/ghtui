//! Where each piece of data comes from: git where it can answer
//! ([`crate::source`]), else GitHub's API; and the cache, which keeps what
//! the API answered.

use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;

use super::{Data, DataKey};
use crate::api::browse::{SearchKind, SearchResults};
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
    let DataKey::Issue(repo, number) = key else { return None };
    let pr = PrRef {
        repo: repo.clone(),
        number: *number,
    };
    Some(Cached {
        value: Data::Issue(None),
        fetched_at: cached(gh, &DataKey::Pr(pr))?.fetched_at,
    })
}

/// `key`'s data from GitHub's API (git reads a thread's lines, and a
/// wiki); for a list, its page after `after`. A first page is kept in
/// the cache, if `key`'s data is ([`DataKey::kept`]). With `early`, a
/// search's page is sent there as soon as it's found, as a first look
/// while its pull requests' checks are read; and a pull request, before
/// how far behind its base it is.
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
        K::Repo(repo) => gh.repo(repo).await?.into(),
        K::Readme(repo) => Data::Readme(gh.readme(repo).await?.map(Box::new)),
        K::Tree(repo, rev, path) => Data::Tree(gh.tree(repo, rev, path).await?),
        K::Blob(repo, rev, path) => gh.blob(repo, rev, path).await?.into(),
        K::Search(kind, query) => {
            let found = |r: &crate::api::browse::SearchResults| {
                if let Some(tx) = early {
                    let _ = tx.send(Msg::fetched_at(
                        key.clone(),
                        Ok(Data::from(r.clone())),
                        Some(crate::store::now()),
                    ));
                }
            };
            let results = gh.search_showing(*kind, query, after, found).await?;
            Data::from(results)
        }
        K::StateCounts(query) => Data::StateCounts(gh.state_counts(query).await?),
        K::Issue(repo, number) => Data::Issue(gh.issue(repo, *number).await?.map(Box::new)),
        K::Pr(pr) => {
            let detail = gh.pull_request_alone(pr).await?;
            // Shown before how far behind its base it is arrives.
            if let Some(tx) = early.filter(|_| detail.state.is_open()) {
                let _ = tx.send(Msg::fetched(key.clone(), Ok(Data::from(detail.clone()))));
            }
            Data::from(gh.with_behind(pr, detail).await)
        }
        K::PrActivity(pr) => gh.pr_activity(pr).await?.into(),
        K::Reviews(pr) => gh.reviews(pr).await?.into(),
        K::InProjects(repo, n) => gh.projects_of(repo, *n).await?.into(),
        K::Lines(repo, commit, path) => {
            let lines = crate::talk::read_lines(git, repo, commit, path).await?;
            Data::Lines(Arc::new(lines))
        }
        K::CommitComments(repo, oid) => gh.commit_comments(repo, oid).await?.into(),
        K::Profile(login) => gh.profile(login).await?.into(),
        K::Files(repo, rev) => {
            let (files, truncated) = gh.file_list(repo, rev).await?;
            Data::Files(Arc::new(files), truncated)
        }
        K::Refs(repo) => gh.refs(repo).await?.into(),
        K::RefIn(repo, spot) => Data::RefIn(gh.ref_in(repo, spot).await?),
        K::LastCommits(repo, rev, path) => {
            let names: Vec<String> = (gh.tree(repo, rev, path).await?.into_iter())
                .map(|e| e.name)
                .collect();
            let found = gh.last_commits(repo, rev, path, &names).await?;
            Data::LastCommits(Arc::new(found))
        }
        K::Commit(repo, oid) => gh.commit(repo, oid).await?.into(),
        K::History(repo, rev, path) => gh.history(repo, rev, path, after).await?.into(),
        K::PrChecks(pr) => gh.pr_checks(pr).await?.into(),
        K::Users(list) => gh.users(list, after).await?.into(),
        K::Forks(repo) => gh.forks(repo, after).await?.into(),
        K::Releases(repo) => gh.releases(repo, after).await?.into(),
        K::Release(repo, tag) => gh.release(repo, tag).await?.into(),
        K::Tags(repo) => gh.tags(repo, after).await?.into(),
        K::Branches(repo) => gh.branches(repo, after).await?.into(),
        K::Wiki(repo, page) => crate::wiki::page(git, repo, page.as_deref()).await?.into(),
        K::Advisories(repo) => gh.advisories(repo.as_ref(), after).await?.into(),
        K::Advisory(repo, ghsa) => gh.advisory(repo.as_ref(), ghsa).await?.into(),
        K::Teams(org) => gh.teams(org, after).await?.into(),
        K::Team(org, slug) => gh.team(org, slug).await?.into(),
        K::Gist(id) => gh.gist(id).await?.into(),
        K::Gists(login) => gh.gists(login, after).await?.into(),
        K::Blame(repo, rev, path) => gh.blame(repo, rev, path).await?.into(),
        K::Compare(repo, spec) => gh.compare(repo, spec).await?.into(),
        K::Deployments(repo, env) => gh.deployments(repo, env.as_deref(), after).await?.into(),
        K::Milestones(repo, closed) => gh.milestones(repo, *closed, after).await?.into(),
        K::Milestone(repo, n) => gh.milestone(repo, *n, after).await?.into(),
        K::Discussions(of, category) => {
            Data::from(gh.discussions(of, category.as_deref(), after).await?)
        }
        K::Discussion(of, n) => gh.discussion(of, *n).await?.into(),
        K::Run(repo, run, attempt) => gh.workflow_run(repo, *run, *attempt).await?.into(),
        K::Job(repo, job) => gh.job(repo, *job).await?.into(),
        K::JobLog(repo, job) => Data::Log(Arc::new(gh.job_log(repo, *job).await?)),
        K::Workflow(repo, file) => gh.workflow(repo, file).await?.into(),
        K::Runs(repo, query) => gh.runs(repo, query, after).await?.into(),
        K::CommitChecks(repo, oid) => gh.commit_checks(repo, oid).await?.into(),
        K::OwnerRepos(login, sort) => gh.owner_repos(login, *sort, after).await?.into(),
        K::Stars(login) => gh.starred(login, after).await?.into(),
        K::Notifications(query) => gh.notifications(query, after).await?.into(),
        K::Sidebar(repo, number) => gh.sidebar(repo, *number).await?.into(),
        K::NewForm(repo) => gh.new_form(repo).await?.into(),
        K::Projects(of) => gh.projects(of, after).await?.into(),
        K::Project(p, view) => gh.project(p, *view, after).await?.into(),
        K::Alerts(repo, kind, query) => gh.alerts(repo, *kind, query, after).await?.into(),
        K::Alert(a) => gh.alert(a).await?.into(),
        K::Subscription(of) => gh.subscription(of).await?.into(),
        K::Settings(repo, page) => gh.settings(repo, *page).await?.into(),
        K::Activity(repo) => gh.activity(repo).await?.into(),
        K::Traffic(repo) => gh.traffic(repo).await?.into(),
        K::Dependencies(repo) => gh.dependencies(repo).await?.into(),
        K::Featured => gh.featured_topics().await?.into(),
        K::Topic(name) => gh.topic(name).await?.into(),
        K::StarLists(login) => gh.star_lists(login).await?.into(),
        K::StarList(login, slug) => gh.star_list(login, slug, after).await?.into(),
        K::MyLists(repo) => gh.my_lists(repo).await?.into(),
        K::Packages(of) => gh.packages(of).await?.into(),
        K::Package(p) => gh.package(p, after).await?.into(),
        K::SavedReplies => gh.saved_replies().await?.into(),
        K::Labels(repo) => gh.labels(repo, after).await?.into(),
        K::Owners => gh.owners().await?.into(),
        K::Templates => gh.templates().await?.into(),
        K::OpenIssues(repo) => gh.open_issues(repo).await?.into(),
        K::MoveTargets(owner) => gh.transfer_targets(owner).await?.into(),
        K::Categories(repo) => gh.categories(repo).await?.into(),
        K::Clone(repo) => {
            let clone = crate::git::local::clone_of(&git.cwd, &repo.owner, &repo.name).await?;
            Data::Clone(clone.map(Box::new))
        }
        K::Head { repo, base, head } => {
            let found = crate::create::head_state(gh, &git.cwd, repo, base, head).await;
            Data::from(found?)
        }
        K::TagLook(repo, tag) => gh.tag_look(repo, tag).await?.into(),
        K::BranchRules(repo, name) => gh.branch_rules(repo, name).await?.into(),
        K::FileLook(repo, rev, path) => gh.file_look(repo, rev, path).await?.into(),
        K::Mentionable(repo) => gh.mentionable(repo).await?.into(),
        K::Unread => Data::Unread(gh.unread().await?),
        K::Suggest(q) => match gh.search(SearchKind::Repos, q, None).await? {
            SearchResults::Repos(r) => Data::Suggestions(r.items.into_iter().take(6).collect()),
            _ => Data::Suggestions(Vec::new()),
        },
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
