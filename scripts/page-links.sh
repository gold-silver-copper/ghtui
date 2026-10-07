#!/bin/sh
# Lists the link shapes on a few dozen real github.com pages, to catch views
# the API descriptions miss (compare, blame, insights...). Run by hand, once
# in a while; it reads one page a second and isn't part of the test suite.
#
#   scripts/page-links.sh > shapes.txt
set -eu

pages="
rust-lang/rust
rust-lang/rust/issues/163852
rust-lang/rust/pull/163831
rust-lang/rust/pull/163831/checks
rust-lang/rust/pull/163831/files
rust-lang/rust/actions
rust-lang/rust/actions/runs/37486756971
rust-lang/rust/actions/runs/37486756971/job/112349439173
rust-lang/rust/releases/tag/1.99.0
rust-lang/rust/commit/8d1a76430406c877b35d0b627e7f796dcf0dfeca
rust-lang/rust/blob/main/README.md
rust-lang/rust/milestones
rust-lang/rust/pulse
rust-lang/rust/graphs/contributors
rust-lang/rust/compare/1.98.0...1.99.0
rust-lang/rust/blame/main/README.md
cli/cli/discussions
cli/cli/discussions/14603
cli/cli/wiki
torvalds
orgs/rust-lang/teams
rust-lang
topics/rust
octocat/Hello-World/security
"
for page in $pages; do
  curl -sSL --max-time 20 "https://github.com/$page" |
    grep -oE 'href="(/|https://github\.com/)[^"#?]*' |
    sed -E 's#^href="##; s#^https://github\.com##' |
    sed -E 's#/[0-9a-f]{40}#/SHA#g; s#/[0-9]+#/N#g'
  sleep 1
done | sort | uniq -c | sort -rn
