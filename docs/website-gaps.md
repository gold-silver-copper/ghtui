# What ghtui can't do that github.com can

ghtui aims to replace the website for daily use. This page lists where it
still falls short, and why. Each gap is in one of four groups:

- **GitHub's API can't do it:** no token can, so ghtui can't either.
- **Partly done:** ghtui does most of it, and the website does the rest.
- **Left out on purpose:** the API could do it, but it's seldom used and
  costly to build.
- **Not live-checked yet:** ghtui does it, but `cargo xtask live` hasn't
  pressed it against real GitHub.

`tests/github_urls.txt` has every kind of github.com link and what ghtui
does with it. A link that opens in the browser gives its reason there. The
README's "Known limitations" covers smaller limits, such as page sizes and
Enterprise Server versions.

## GitHub's API can't do it

| Gap | Why |
| --- | --- |
| Attaching an image or a file to a comment | Only a browser signed in to github.com can upload one. The REST and GraphQL APIs have no upload, so the composer says so. |
| The threads you're subscribed to, as a list | The API answers for one thread at a time; `:watch` changes one. The repositories you watch do have a list (`/watching`). |
| Saved issue views | Not in the API. |
| A repository's attestations | The API finds an attestation only by an artifact's digest, so it can't list a repository's. |
| A repository's dependents (who depends on it) | Not in the API. |
| Declining an organization's invitation | The API can only accept one. A repository's invitation can be declined. |
| Saved replies: making, changing or deleting one | Not in the API. ghtui inserts them; github.com/settings/replies changes them. |
| A running job's live log | The API has no log until the job ends. ghtui follows its steps meanwhile. |
| A job's summary (`$GITHUB_STEP_SUMMARY`) | Kept out of the REST and GraphQL APIs. |
| Trending repositories | The API doesn't say which repositories got stars lately. Explore lists new and most-starred repositories instead. |
| A classic project (`/projects/1` on a repository) | Classic projects are gone from the API. |
| Searching wikis | GitHub's search API has no wiki search. |
| Collections (`/collections`) | Not in the API. |
| Pages that need a signed-in browser | Login, sign-up, token and profile settings, importing a repository, Codespaces, Copilot tasks and spaces, enterprise accounts, app installations and connections, code scanning's tool settings, secret-scanning exemptions, and a project's settings. These are marked `session` in the link corpus. |
| Packages without `read:packages` | gh doesn't grant that scope by default. ghtui names it, along with the `gh auth refresh` command that adds it. |

## Partly done

| Gap | What's left to the website |
| --- | --- |
| An organization's Actions secrets and variables | They're listed on each repository's page. Changing them is left out, because choosing which repositories may use one is a larger form. |
| Custom properties that only an organization's admins may set | They're left out of the form, even for an organization admin: GitHub doesn't tell a repository who its organization's admins are. |
| GitHub Pages for maintainers | ghtui offers Pages changes only to admins, because the API says only whether you're an admin. GitHub also lets maintainers make them. |
| A repository security advisory | ghtui writes a draft with GitHub's fields and lists the drafts. Credits, more than one package, and publishing are done on github.com. |
| Your status | It clears after 30 minutes, an hour, 4 hours, a day or a week. GitHub's "today" and "this week" depend on your time zone, which ghtui doesn't know. A status limited to an organization's members is set on github.com. |
| Resolving conflicts | ghtui makes a merge commit, as GitHub's conflict editor does. A rebase is done in your clone. It needs git 2.38 or newer. |
| A team's page | It shows the first 50 members and the first 50 repositories. |
| A list of starred repositories (`:lists`) | A new list is public. Its name, description and privacy are changed on github.com. |
| The Actions settings page (`/settings/actions`) | The REST API covers Actions permissions, but ghtui has no page for them yet (marked `rare`). |

## Left out on purpose

These are marked `rare` in the link corpus. The API could do most of
them, but they're seldom used day to day, and each one is costly for what
it shows:

| Page | Why it's left out |
| --- | --- |
| Pulse | It's a summary built from several searches. |
| Activity | The REST activity log exists, but is seldom needed. |
| Code frequency | It needs lines added and removed per week. Git would have to fetch every file's contents in the cache's blobless clone, and GitHub's statistics cover only repositories with fewer than 10,000 commits. |
| The network graph | It isn't in the API, and drawing every fork's branches is costly. |
| Sponsors, GitHub Apps' pages, gist Discover | Not daily use. |

## Not live-checked yet

`cargo xtask live` presses each write in the real app and checks GitHub's
own answer with `gh api`. Every write has such a check. The checks below
are written, but have only taken their skip path so far, because the test
account lacks what they need. Each one prints what it needs when it
skips:

| Check | What it presses | What it needs |
| --- | --- | --- |
| `orgs` | Making a team, giving it a repository, changing its role, removing it | An organization the account owns, with a repository, and the `admin:org` scope |
| `members` | Inviting by email and cancelling; changing a member's role; adding, changing and removing a team's member; removing a member | The same, plus `GHTUI_LIVE_SECOND_TOKEN` for the member steps |
| `invitations` | Declining and accepting a repository invitation; joining an organization | `GHTUI_LIVE_SECOND_TOKEN`: a classic token of a second account you control, with `repo` and `admin:org`. The organization part also needs an organization the account owns. |
| `properties` | Setting a custom property | An organization the account owns, with a repository |
| `issue-types` | Setting and clearing an issue's type | An organization the account owns, with a repository |
| `people` (following, your status) | Following and unfollowing; setting and clearing your status | The `user:follow` and `user` scopes |

Two parts are unconfirmed until those checks run against GitHub: whether
GitHub accepts an organization invitation to an `example.com` address,
and exactly which scopes the second account's token needs.

To run them, once the account owns an organization and has the scopes
(`gh auth refresh -s admin:org,user,user:follow`):

```sh
cd ~/code/ghtui && GHTUI_LIVE_SECOND_TOKEN=<a second account's token> cargo xtask live orgs members invitations properties issue-types people
```
