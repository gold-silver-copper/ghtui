//! GitHub's own answer, through `gh api` with the owner's gh login.
//! Writes go only to [`REPO`].

use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// The only repository the check writes to.
pub const REPO: &str = "gold-silver-copper/ghtui-test-writes";
pub const OWNER: &str = "gold-silver-copper";
pub const NAME: &str = "ghtui-test-writes";
/// Branches the check pushes start with this.
pub const BRANCH_PREFIX: &str = "ghtui-live-";

/// `gh api ARGS…`, with `body` as the JSON request body.
pub fn api(args: &[&str], body: Option<&Value>) -> Result<Value> {
    call(None, args, body)
}

/// A request as another account: `token` is its `GH_TOKEN`, never shown.
pub fn send_as(token: &str, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    call(Some(token), &["-X", method, path], body)
}

fn call(token: Option<&str>, args: &[&str], body: Option<&Value>) -> Result<Value> {
    let mut cmd = Command::new("gh");
    cmd.arg("api").args(args);
    if let Some(token) = token {
        cmd.env("GH_TOKEN", token);
    }
    if body.is_some() {
        cmd.args(["--input", "-"]);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().context("cannot run gh")?;
    if let Some(body) = body
        && let Some(mut stdin) = child.stdin.take()
    {
        stdin.write_all(body.to_string().as_bytes())?;
    }
    drop(child.stdin.take());
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "gh api {} failed: {}{}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim(),
            String::from_utf8_lossy(&out.stdout).trim()
        );
    }
    if out.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&out.stdout).with_context(|| format!("gh api {}", args.join(" ")))
}

pub fn get(path: &str) -> Result<Value> {
    api(&[path], None)
}

/// A request with a method and maybe a body: `send("PATCH", "repos/…", Some(&json))`.
pub fn send(method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
    api(&["-X", method, path], body)
}

/// Every page of a REST list.
pub fn rest_list(path: &str) -> Result<Vec<Value>> {
    let pages = api(&["--paginate", "--slurp", path], None)?;
    Ok(pages
        .as_array()
        .map(|pages| {
            pages
                .iter()
                .filter_map(Value::as_array)
                .flatten()
                .cloned()
                .collect()
        })
        .unwrap_or_default())
}

/// A GraphQL query's `data`; its errors are an error.
pub fn graphql(query: &str, variables: Value) -> Result<Value> {
    let answer = api(
        &["graphql"],
        Some(&json!({"query": query, "variables": variables})),
    )?;
    if let Some(errors) = answer.get("errors") {
        bail!("GraphQL: {errors}");
    }
    Ok(answer["data"].clone())
}

/// The account gh is logged in as, and its token's scopes.
pub fn account() -> Result<(String, Vec<String>)> {
    let out = Command::new("gh")
        .args(["api", "-i", "user"])
        .output()
        .context("cannot run gh: the live check needs it, logged in")?;
    if !out.status.success() {
        bail!(
            "gh api user failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let scopes = text
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("x-oauth-scopes").then(|| {
                value
                    .split(',')
                    .map(|s| s.trim().to_owned())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
        })
        .unwrap_or_default();
    let body = text
        .split("\r\n\r\n")
        .nth(1)
        .or_else(|| text.split("\n\n").nth(1));
    let user: Value = serde_json::from_str(body.unwrap_or_default().trim())?;
    let login = user["login"].as_str().unwrap_or_default().to_owned();
    if login != OWNER {
        bail!("gh is logged in as {login}; the live check writes to {REPO}, so it needs {OWNER}");
    }
    Ok((login, scopes))
}

/// The token, for the stand-in Enterprise Server's `GH_ENTERPRISE_TOKEN`.
pub fn token() -> Result<String> {
    let out = Command::new("gh").args(["auth", "token"]).output()?;
    if !out.status.success() {
        bail!("gh auth token failed");
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub fn issue(number: u64) -> Result<Value> {
    get(&format!("repos/{REPO}/issues/{number}"))
}

pub fn delete_issue(number: u64) -> Result<()> {
    let id = issue(number)?["node_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    graphql(
        "mutation($id: ID!) { deleteIssue(input: {issueId: $id}) { clientMutationId } }",
        json!({"id": id}),
    )?;
    Ok(())
}

pub fn close_pr(number: u64) -> Result<()> {
    send(
        "PATCH",
        &format!("repos/{REPO}/pulls/{number}"),
        Some(&json!({"state": "closed"})),
    )?;
    Ok(())
}

pub fn delete_branch(name: &str) -> Result<()> {
    send(
        "DELETE",
        &format!("repos/{REPO}/git/refs/heads/{name}"),
        None,
    )?;
    Ok(())
}

/// A new issue; its number.
pub fn create_issue(title: &str, body: &str) -> Result<u64> {
    let issue = send(
        "POST",
        &format!("repos/{REPO}/issues"),
        Some(&json!({"title": title, "body": body})),
    )?;
    issue["number"].as_u64().context("no issue number")
}

/// Closes the branch's open pull requests and deletes it, if it's there.
pub fn drop_branch(branch: &str) -> Result<()> {
    for pr in rest_list(&format!(
        "repos/{REPO}/pulls?head={OWNER}:{branch}&state=open"
    ))? {
        if let Some(number) = pr["number"].as_u64() {
            close_pr(number)?;
        }
    }
    match get(&format!("repos/{REPO}/branches/{branch}")) {
        Ok(_) => delete_branch(branch),
        Err(_) => Ok(()),
    }
}

/// Seconds since 1970, now.
pub fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// `2026-10-12T14:03:56Z`, as GitHub writes times (they compare as text).
pub fn iso8601(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso8601() {
        assert_eq!(super::iso8601(0), "1970-01-01T00:00:00Z");
        assert_eq!(super::iso8601(1_791_813_836), "2026-10-12T14:03:56Z");
    }
}
