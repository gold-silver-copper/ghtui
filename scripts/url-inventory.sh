#!/bin/sh
# Prints a real github.com URL for every GraphQL type that has one, from
# public objects in busy repositories, for crates/app/tests/github_urls.toml.
# Run by hand (needs `gh auth`); not part of the test suite. Projects (v2)
# need the read:project scope, which gh tokens lack by default.
#
#   scripts/url-inventory.sh > urls.json
set -eu

gh api graphql -f query='
query {
  repository(owner: "rust-lang", name: "rust") {
    url
    licenseInfo { url }
    codeOfConduct { url }
    contributingGuidelines { url }
    contactLinks { url }
    fundingLinks { url }
    mergeQueue { url }
    issues(last: 1, states: CLOSED) {
      nodes {
        url
        comments(first: 1) { nodes { url } }
        timelineItems(last: 20, itemTypes: [CLOSED_EVENT, CROSS_REFERENCED_EVENT]) {
          nodes { __typename ... on ClosedEvent { url } ... on CrossReferencedEvent { url } }
        }
      }
    }
    pullRequests(last: 1, states: MERGED) {
      nodes {
        url
        reviews(first: 1) { nodes { url comments(first: 1) { nodes { url } } } }
        commits(first: 1) { nodes { url commit { url } } }
        timelineItems(last: 30, itemTypes: [MERGED_EVENT, READY_FOR_REVIEW_EVENT, CONVERT_TO_DRAFT_EVENT, REVIEW_DISMISSED_EVENT]) {
          nodes {
            __typename
            ... on MergedEvent { url }
            ... on ReadyForReviewEvent { url }
            ... on ConvertToDraftEvent { url }
            ... on ReviewDismissedEvent { url }
          }
        }
      }
    }
    milestones(first: 1, states: OPEN) { nodes { url } }
    releases(first: 1) { nodes { url releaseAssets(first: 1) { nodes { url downloadUrl } } } }
    labels(first: 1) { nodes { url } }
    repositoryTopics(first: 1) { nodes { url } }
    object(expression: "HEAD") {
      ... on Commit {
        url
        comments(first: 1) { nodes { url } }
        checkSuites(first: 5) {
          nodes {
            url
            app { url }
            workflowRun { url workflow { url } file { url } }
            checkRuns(first: 1) { nodes { url permalink detailsUrl } }
          }
        }
      }
    }
  }
  cli: repository(owner: "cli", name: "cli") {
    discussions(first: 1) { nodes { url comments(first: 1) { nodes { url } } } }
    object(expression: "HEAD") { ... on Commit { status { contexts { targetUrl } } } }
  }
  org: organization(login: "github") {
    url
    teams(first: 1) { nodes { url } }
    sponsorsListing { url }
  }
  user(login: "torvalds") {
    url
    socialAccounts(first: 1) { nodes { url } }
  }
  gists: user(login: "octocat") { gists(first: 1) { nodes { url } } }
  license(key: "mit") { url }
  securityAdvisories(first: 1) { nodes { ghsaId permalink references { url } } }
  reviewed: repository(owner: "rust-lang", name: "rust") {
    pullRequest(number: 163831) {
      reviews(first: 3) { nodes { url comments(first: 1) { nodes { url } } } }
      files(first: 1) { nodes { path } }
    }
  }
  marketplaceListing(slug: "dependabot-preview") { url }
  topic(name: "rust") { name }
}' "$@"
