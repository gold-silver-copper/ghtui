//! The pages GitHub's website has beyond code and conversations:
//! Insights (contributors and commit activity counted by git, traffic,
//! the dependency graph), Explore, starred lists (`,B` and `,D`),
//! packages (`,D` on a version, or the scope it takes), and saved
//! replies in the composer (`ctrl-g`, or the scope it takes).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::live::{Check, Cx, Launch, NARROW, gh, until};

pub const INSIGHTS: Check = Check::new(
    "insights",
    "Insights at 90 columns: contributors and commit activity counted by git, traffic and the dependency graph as GitHub says",
    insights,
);

pub const EXPLORE: Check = Check::new(
    "explore",
    "Explore's new and most starred Rust repositories are GitHub search's, and a topic's page its own",
    explore,
);

pub const LISTS: Check = Check::new(
    "lists",
    ",B makes a ghtui-live- list with the test repository in it and takes it out; ,D on the list's page deletes it",
    lists,
)
.sweep(sweep_lists);

pub const PACKAGES: Check = Check::new(
    "packages",
    "the Packages tab names the read:packages scope; with the packages scopes, ,D deletes a version of a ghtui-live- image",
    packages,
)
.sweep(sweep_packages);

pub const REPLIES: Check = Check::new(
    "saved-replies",
    "ctrl-g in the composer lists your saved replies and ↵ puts one in, or it names the read:user scope",
    replies,
);

const GITHUB: Duration = Duration::from_secs(60);
/// The cache's clone is made the first time a page reads with git.
const CLONE: Duration = Duration::from_secs(120);

fn has(cx: &Cx, scope: &str) -> bool {
    cx.env.scopes.iter().any(|s| s == scope)
}

// ---- Insights ----------------------------------------------------------------------------

fn insights(cx: &mut Cx) -> Result<()> {
    cx.step("contributors: main's commits, merges left out, counted by git")?;
    let commits = gh::rest_list(&format!("repos/{}/commits?sha=main&per_page=100", gh::REPO))?;
    let counted = (commits.iter())
        .filter(|c| c["parents"].as_array().is_some_and(|p| p.len() < 2))
        .count();
    let url = format!("https://github.com/{}/graphs/contributors", gh::REPO);
    let ghtui = cx.launch(Launch::new(&[&url]).size(NARROW))?;
    ghtui.wait_for("Contributors · Commit activity · Traffic · Dependency graph")?;
    // GitHub's statistics answer while the clone is made; then git does.
    ghtui
        .wait_until("the page read by git", CLONE, |screen| {
            screen.contains("read by git").then_some(())
        })
        .or_else(|_| {
            ghtui.keys("r")?;
            ghtui.wait_for_within("read by git", CLONE).map(drop)
        })?;
    let noun = if counted == 1 { "commit" } else { "commits" };
    ghtui.wait_for(&format!("Commits over time  {counted} {noun} since"))?;
    ghtui.wait_for("Counted by git on the default branch")?;

    cx.step("commit activity: the last 52 weeks, and by day of the week")?;
    let url = format!("https://github.com/{}/graphs/commit-activity", gh::REPO);
    let ghtui = cx.launch(Launch::new(&[&url]).size(NARROW))?;
    ghtui.wait_for("Commits in the last 52 weeks")?;
    ghtui.wait_for("By day of the week, in the last 52 weeks")?;

    cx.step("traffic: views and clones as GitHub counts them")?;
    let views = gh::get(&format!("repos/{}/traffic/views", gh::REPO))?;
    let clones = gh::get(&format!("repos/{}/traffic/clones", gh::REPO))?;
    let url = format!("https://github.com/{}/graphs/traffic", gh::REPO);
    let ghtui = cx.launch(Launch::new(&[&url]).size(NARROW))?;
    ghtui.wait_for(&format!(
        "Views  {} · {} unique visitors",
        views["count"], views["uniques"]
    ))?;
    ghtui.wait_for(&format!(
        "Git clones  {} · {} unique cloners",
        clones["count"], clones["uniques"]
    ))?;

    cx.step("the dependency graph: GitHub's manifests, and a link to the dependents")?;
    let graph = gh::graphql(
        "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { dependencyGraphManifests(first: 1) { totalCount nodes { filename } } } }",
        json!({"owner": gh::OWNER, "name": gh::NAME}),
    )?;
    let manifests = &graph["repository"]["dependencyGraphManifests"];
    let url = format!("https://github.com/{}/network/dependencies", gh::REPO);
    let ghtui = cx.launch(Launch::new(&[&url]).size(NARROW))?;
    ghtui.wait_for("aren't in GitHub's API")?;
    if manifests["totalCount"] == 0 {
        ghtui.wait_for("GitHub found no manifests")?;
    } else {
        let first = manifests["nodes"][0]["filename"]
            .as_str()
            .unwrap_or_default();
        ghtui.wait_for(first)?;
    }
    Ok(())
}

// ---- Explore -----------------------------------------------------------------------------

fn explore(cx: &mut Cx) -> Result<()> {
    cx.step("Explore's Rust list this week is GitHub search's, most starred first")?;
    let week_ago = gh::iso8601(gh::epoch().saturating_sub(7 * 86_400));
    let day = week_ago.get(..10).unwrap_or(&week_ago);
    let found = gh::get(&format!(
        "search/repositories?q=created:%3E{day}+language:rust&sort=stars&order=desc&per_page=1"
    ))?;
    let first = found["items"][0]["full_name"]
        .as_str()
        .context("GitHub's search found no new Rust repository")?
        .to_owned();
    let ghtui =
        cx.launch(Launch::new(&["https://github.com/trending/rust?since=weekly"]).size(NARROW))?;
    ghtui.wait_for("Trending ranks repositories")?;
    ghtui.wait_for("New and most starred, this week")?;
    ghtui.wait_for(&first)?;
    // GitHub's featured topics, at the bottom (by title, the last).
    let featured = gh::get("search/topics?q=is:featured&per_page=100")?;
    let mut names: Vec<(String, String)> = (featured["items"].as_array())
        .map(|items| {
            items
                .iter()
                .map(|t| {
                    let name = t["name"].as_str().unwrap_or_default().to_owned();
                    let title = t["display_name"].as_str().unwrap_or(&name).to_lowercase();
                    (title, name)
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    let last = names
        .last()
        .map(|(_, n)| n.clone())
        .context("GitHub features no topic")?;
    ghtui.keys("G")?;
    ghtui.wait_for(&format!("#{last}"))?;

    cx.step("a topic: its stars, related topics and repositories")?;
    let ghtui = cx.launch(Launch::new(&["https://github.com/topics/rust"]).size(NARROW))?;
    ghtui.wait_for("Rust")?;
    ghtui.wait_for("Related")?;
    ghtui.wait_for("Repositories")?;
    Ok(())
}

// ---- starred lists -----------------------------------------------------------------------

/// Your lists: their IDs, names and slugs, and the repositories in each.
fn my_lists() -> Result<Vec<Value>> {
    let data = gh::graphql(
        "query { viewer { lists(first: 100) { nodes { id name slug items(first: 100) { nodes { ... on Repository { nameWithOwner } } } } } } }",
        json!({}),
    )?;
    Ok(data["viewer"]["lists"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

fn list_named(name: &str) -> Result<Option<Value>> {
    Ok(my_lists()?.into_iter().find(|l| l["name"] == name))
}

fn holds_repo(list: &Value) -> bool {
    list["items"]["nodes"]
        .as_array()
        .is_some_and(|items| items.iter().any(|i| i["nameWithOwner"] == gh::REPO))
}

fn delete_list(id: &str) -> Result<()> {
    gh::graphql(
        "mutation($list: ID!) { deleteUserList(input: {listId: $list}) { clientMutationId } }",
        json!({"list": id}),
    )?;
    Ok(())
}

fn starred() -> Result<bool> {
    match gh::get(&format!("user/starred/{}", gh::REPO)) {
        Ok(_) => Ok(true),
        Err(err) if format!("{err:#}").contains("Not Found") => Ok(false),
        Err(err) => Err(err),
    }
}

fn lists(cx: &mut Cx) -> Result<()> {
    let name = format!("{}{}", gh::BRANCH_PREFIX, cx.env.run_id);
    // Putting a repository in a list stars it, as on GitHub.
    let was_starred = starred()?;
    cx.defer("put the test repository's star back", move || {
        if starred()? != was_starred {
            let method = if was_starred { "PUT" } else { "DELETE" };
            gh::send(method, &format!("user/starred/{}", gh::REPO), None)?;
        }
        Ok(())
    });
    let undo = name.clone();
    cx.defer(format!("delete the list {name}"), move || {
        if let Some(list) = list_named(&undo)? {
            delete_list(list["id"].as_str().unwrap_or_default())?;
        }
        Ok(())
    });
    let ghtui = cx.ghtui(&[gh::REPO])?;
    ghtui.wait_for("Pull requests")?;
    ghtui.wait_followed()?;

    cx.step(",B makes the list, typed, with the repository in it")?;
    ghtui.keys(", B")?;
    ghtui.wait_for("Your lists")?;
    ghtui.type_text(&name)?;
    ghtui.wait_for(&format!("Make the list “{name}” and add it"))?;
    ghtui.keys("Enter")?;
    if !has(cx, "user") {
        ghtui.wait_for("gh auth refresh -s user")?;
        ensure!(
            list_named(&name)?.is_none(),
            "the list was made without the user scope"
        );
        return Err(Cx::skip(
            "changing starred lists takes the token's user scope (gh auth refresh -s user), and ghtui said so",
        ));
    }
    let list = until("the list on GitHub, with the repository", GITHUB, || {
        Ok(list_named(&name)?.filter(holds_repo))
    })?;
    ghtui.wait_followed()?;

    cx.step(",B again shows it checked; ↵ takes the repository out")?;
    ghtui.keys(", B")?;
    ghtui.type_text(&name)?;
    ghtui.wait_for(&format!("✓ {name}"))?;
    ghtui.keys("Enter")?;
    until("the repository out of the list on GitHub", GITHUB, || {
        Ok(list_named(&name)?.filter(|l| !holds_repo(l)))
    })?;
    ghtui.wait_followed()?;

    cx.step("the list's page; ,D deletes the list, after asking")?;
    let slug = list["slug"].as_str().unwrap_or_default();
    let url = format!("https://github.com/stars/{}/lists/{slug}", cx.env.login);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for("No repositories in this list yet.")?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("Delete the list “{name}”?"))?;
    ghtui.keys("Enter")?;
    until("the list gone from GitHub", GITHUB, || {
        Ok(list_named(&name)?.is_none().then_some(()))
    })?;
    ghtui.wait_for(&format!("Deleted the list “{name}”"))?;
    Ok(())
}

fn sweep_lists() -> Result<()> {
    for list in my_lists()? {
        if list["name"]
            .as_str()
            .is_some_and(|n| n.starts_with(gh::BRANCH_PREFIX))
        {
            delete_list(list["id"].as_str().unwrap_or_default())?;
        }
    }
    Ok(())
}

// ---- packages ----------------------------------------------------------------------------

fn packages(cx: &mut Cx) -> Result<()> {
    if !has(cx, "read:packages") && !has(cx, "write:packages") {
        cx.step("without read:packages, the Packages tab says the scope it takes")?;
        let url = format!("https://github.com/{}?tab=packages", cx.env.login);
        let ghtui = cx.launch(Launch::new(&[&url]).size(NARROW))?;
        ghtui.wait_for("gh auth refresh -s read:packages")?;
        return Err(Cx::skip(
            "packages take the token's read:packages, write:packages and delete:packages scopes (gh auth refresh -s read:packages,write:packages,delete:packages); ghtui named the one missing",
        ));
    }
    if !has(cx, "write:packages") || !has(cx, "delete:packages") {
        return Err(Cx::skip(
            "making and deleting the check's image takes write:packages and delete:packages (gh auth refresh -s write:packages,delete:packages)",
        ));
    }
    let name = format!("{}{}", gh::BRANCH_PREFIX, cx.env.run_id);
    let undo = name.clone();
    cx.defer(format!("delete the package {name}"), move || {
        delete_package(&undo)
    });
    cx.step("two versions of a tiny image, pushed to ghcr.io")?;
    let login = cx.env.login.to_lowercase();
    push_image(&cx.env.work, &login, &name, "one")?;
    push_image(&cx.env.work, &login, &name, "two")?;
    let versions = format!("user/packages/container/{name}/versions");
    until("both versions on GitHub", GITHUB, || {
        Ok((gh::rest_list(&versions)?.len() == 2).then_some(()))
    })?;

    cx.step("the package's page: how to pull it; ,D on a version deletes it, after asking")?;
    let url = format!(
        "https://github.com/users/{}/packages/container/package/{name}",
        cx.env.login
    );
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for(&format!("docker pull ghcr.io/{login}/{name}:"))?;
    ghtui.keys("/ one Enter")?;
    ghtui.keys(", D")?;
    ghtui.wait_for(&format!("of {name}?"))?;
    ghtui.keys("Enter")?;
    let left = until("one version left on GitHub", GITHUB, || {
        let left = gh::rest_list(&versions)?;
        Ok((left.len() == 1).then_some(left))
    })?;
    let tags = &left[0]["metadata"]["container"]["tags"];
    ensure!(
        tags.as_array()
            .is_some_and(|t| t.iter().any(|t| t == "two")),
        "GitHub kept {tags}, not the version tagged two"
    );
    ghtui.wait_followed()?;
    Ok(())
}

fn delete_package(name: &str) -> Result<()> {
    match gh::send("DELETE", &format!("user/packages/container/{name}"), None) {
        Err(err) if !format!("{err:#}").contains("Not Found") => Err(err),
        _ => Ok(()),
    }
}

fn sweep_packages() -> Result<()> {
    let Ok(found) = gh::rest_list("user/packages?package_type=container&per_page=100") else {
        // Without read:packages there's nothing the check could have made.
        return Ok(());
    };
    for p in found {
        if let Some(name) = p["name"].as_str()
            && name.starts_with(gh::BRANCH_PREFIX)
        {
            delete_package(name)?;
        }
    }
    Ok(())
}

/// Pushes an empty image, tagged `tag`, as `owner/name` to ghcr.io,
/// with the registry's own API (no docker needed). Its config says the
/// tag, so each tag is a version of its own.
fn push_image(work: &Path, owner: &str, name: &str, tag: &str) -> Result<()> {
    let token = gh::token()?;
    let basic = base64(format!("{owner}:{token}").as_bytes());
    let scope = format!("repository:{owner}/{name}:pull,push");
    let answer = curl(
        work,
        &[
            "-sS",
            "--fail-with-body",
            &format!("https://ghcr.io/token?scope={scope}&service=ghcr.io"),
        ],
        &format!("Authorization: Basic {basic}"),
        None,
    )?;
    let bearer: Value = serde_json::from_slice(&answer)?;
    let bearer = bearer["token"].as_str().context("ghcr.io gave no token")?;
    let auth = format!("Authorization: Bearer {bearer}");
    let base = format!("https://ghcr.io/v2/{owner}/{name}");
    let config = json!({
        "architecture": "amd64",
        "os": "linux",
        "config": {"Labels": {"ghtui-live": tag}},
        "rootfs": {"type": "layers", "diff_ids": []},
    })
    .to_string();
    let digest = format!("sha256:{}", sha256(config.as_bytes())?);
    let head = curl(
        work,
        &[
            "-sS",
            "--fail-with-body",
            "-D",
            "-",
            "-o",
            "/dev/null",
            "-X",
            "POST",
            &format!("{base}/blobs/uploads/"),
        ],
        &auth,
        None,
    )?;
    let head = String::from_utf8_lossy(&head);
    let location = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.eq_ignore_ascii_case("location")
                .then(|| v.trim().to_owned())
        })
        .context("ghcr.io gave no upload location")?;
    let location = if location.starts_with('/') {
        format!("https://ghcr.io{location}")
    } else {
        location
    };
    let join = if location.contains('?') { '&' } else { '?' };
    curl(
        work,
        &[
            "-sS",
            "--fail-with-body",
            "-X",
            "PUT",
            "-H",
            "Content-Type: application/octet-stream",
            "--data-binary",
            "@-",
            &format!("{location}{join}digest={digest}"),
        ],
        &auth,
        Some(config.as_bytes()),
    )?;
    let manifest = json!({
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {
            "mediaType": "application/vnd.oci.image.config.v1+json",
            "digest": digest,
            "size": config.len(),
        },
        "layers": [],
    })
    .to_string();
    curl(
        work,
        &[
            "-sS",
            "--fail-with-body",
            "-X",
            "PUT",
            "-H",
            "Content-Type: application/vnd.oci.image.manifest.v1+json",
            "--data-binary",
            "@-",
            &format!("{base}/manifests/{tag}"),
        ],
        &auth,
        Some(manifest.as_bytes()),
    )?;
    Ok(())
}

/// Runs curl with `header` (a secret) from a config file only this run
/// can read, never on its command line; `body` goes to its stdin (`@-`
/// reads it). Its output.
fn curl(work: &Path, args: &[&str], header: &str, body: Option<&[u8]>) -> Result<Vec<u8>> {
    let config = work.join("curl-header");
    let quoted = header.replace('\\', "\\\\").replace('"', "\\\"");
    std::fs::write(&config, format!("header = \"{quoted}\"\n"))?;
    let run = || -> Result<std::process::Output> {
        let mut child = Command::new("curl")
            .arg("-K")
            .arg(&config)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("can't run curl")?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body.unwrap_or_default())?;
        }
        Ok(child.wait_with_output()?)
    };
    let out = run();
    let _ = std::fs::remove_file(&config);
    let out = out?;
    if !out.status.success() {
        bail!(
            "curl {}: {}{}",
            args.last().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim(),
            String::from_utf8_lossy(&out.stdout).trim()
        );
    }
    Ok(out.stdout)
}

fn sha256(bytes: &[u8]) -> Result<String> {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .context("can't run sha256sum")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(bytes)?;
    }
    let out = child.wait_with_output()?;
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned())
}

fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let at = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap_or(0);
                out.push(char::from(ABC.get(at).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

// ---- saved replies -----------------------------------------------------------------------

fn replies(cx: &mut Cx) -> Result<()> {
    let issue = 8;
    let ghtui = cx.ghtui(&[&format!("{}#{issue}", gh::REPO)])?;
    ghtui.wait_for("Issue to close completed")?;
    ghtui.wait_followed()?;
    cx.step("c opens the composer; ctrl-g lists your saved replies")?;
    ghtui.keys("c")?;
    ghtui.wait_for("ctrl-g saved replies")?;
    ghtui.keys("C-g")?;
    ghtui.wait_for("Saved replies ⌕")?;
    ghtui.wait_for("github.com/settings/replies")?;
    if !has(cx, "read:user") && !has(cx, "user") {
        ghtui.wait_for("gh auth refresh -s read:user adds it")?;
        ghtui.keys("Escape Escape")?;
        return Err(Cx::skip(
            "saved replies take the token's read:user scope (gh auth refresh -s read:user), and ghtui said so",
        ));
    }
    let data = gh::graphql(
        "query { viewer { savedReplies(first: 1) { nodes { title body } } } }",
        json!({}),
    )?;
    let Some(reply) = data["viewer"]["savedReplies"]["nodes"].get(0).cloned() else {
        ghtui.wait_for("You have no saved replies")?;
        ghtui.keys("Escape Escape")?;
        return Err(Cx::skip(
            "you have no saved replies to insert (make one at github.com/settings/replies)",
        ));
    };
    cx.step("↵ puts the first one in the composer")?;
    let title = reply["title"].as_str().unwrap_or_default();
    let body = reply["body"].as_str().unwrap_or_default();
    ghtui.wait_for(title)?;
    ghtui.type_text(title)?;
    ghtui.keys("Enter")?;
    ghtui.wait_gone("Saved replies ⌕")?;
    let first_line = body.lines().next().unwrap_or_default();
    let shown: String = first_line.chars().take(30).collect();
    ghtui.wait_for(&shown)?;
    // Nothing's posted: esc twice throws the text away.
    ghtui.keys("Escape Escape")?;
    Ok(())
}
