//! Discussions, on a throwaway question in the test repository's Q&A
//! category: `c` writes an answer, `c` on it replies in its thread, `,!`
//! marks it the answer (shown at the top too), `,+` reacts to it. Skipped
//! when the repository has Discussions off.

use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::live::{Check, Cx, gh, until};

pub const CHECK: Check = Check::new(
    "discussion",
    "a question: c answers, c on the answer replies, ,! marks it the answer, ,+ reacts",
    run,
)
.sweep(sweep);

const GITHUB: Duration = Duration::from_secs(30);

fn run(cx: &mut Cx) -> Result<()> {
    let repo = gh::graphql(
        "query($o: String!, $n: String!) { repository(owner: $o, name: $n) { id hasDiscussionsEnabled
            discussionCategories(first: 20) { nodes { id name isAnswerable } } } }",
        json!({"o": gh::OWNER, "n": gh::NAME}),
    )?;
    let repo = &repo["repository"];
    if repo["hasDiscussionsEnabled"] != true {
        return Err(Cx::skip(format!(
            "{} has Discussions off (gh api -X PATCH repos/{} -F has_discussions=true)",
            gh::REPO,
            gh::REPO
        )));
    }
    let category = repo["discussionCategories"]["nodes"]
        .as_array()
        .and_then(|c| c.iter().find(|c| c["isAnswerable"] == true))
        .and_then(|c| c["id"].as_str())
        .context("the repository has no category that takes answers")?
        .to_owned();

    cx.step("a question, made with GraphQL")?;
    let title = cx.mark("discussion");
    let made = gh::graphql(
        "mutation($r: ID!, $c: ID!, $t: String!) { createDiscussion(input: {repositoryId: $r, categoryId: $c, title: $t, body: \"A question from the live check?\"}) { discussion { id number } } }",
        json!({"r": repo["id"], "c": category, "t": title}),
    )?;
    let id = made["createDiscussion"]["discussion"]["id"]
        .as_str()
        .context("no discussion id")?
        .to_owned();
    let number = made["createDiscussion"]["discussion"]["number"]
        .as_u64()
        .context("no discussion number")?;
    let undo = id.clone();
    cx.defer("delete the discussion", move || delete(&undo));
    let url = format!("https://github.com/{}/discussions/{number}", gh::REPO);
    let ghtui = cx.ghtui(&[&url])?;
    ghtui.wait_for(&title)?;
    ghtui.wait_followed()?;

    cx.step("c writes an answer")?;
    let answer = format!("An answer from the live check {}", cx.env.run_id);
    ghtui.keys("c")?;
    ghtui.wait_for(&format!("Answer on “{title}”"))?;
    ghtui.type_text(&answer)?;
    ghtui.keys("C-s")?;
    let comment = until("the answer on GitHub", GITHUB, || {
        Ok(comments(&id)?
            .into_iter()
            .find(|c| c["body"] == answer.as_str()))
    })?;
    let comment_id = comment["id"].as_str().unwrap_or_default().to_owned();
    ghtui.wait_for(&answer)?;

    ghtui.wait_followed()?;
    cx.step("c on the answer replies in its thread")?;
    let selected = format!("⟦▌{answer}");
    ghtui.select("gg }", "the answer", |s| s.contains(&selected))?;
    let reply = format!("A reply from the live check {}", cx.env.run_id);
    ghtui.keys("c")?;
    ghtui.wait_for("open in $EDITOR")?;
    ghtui.type_text(&reply)?;
    ghtui.keys("C-s")?;
    until("the reply on GitHub", GITHUB, || {
        Ok(comments(&id)?
            .iter()
            .find(|c| c["id"] == comment_id.as_str())
            .is_some_and(|c| {
                c["replies"]["nodes"]
                    .as_array()
                    .is_some_and(|r| r.iter().any(|r| r["body"] == reply.as_str()))
            })
            .then_some(()))
    })?;
    ghtui.wait_for(&reply)?;

    ghtui.wait_followed()?;
    cx.step(",! marks it the answer")?;
    ghtui.select("gg }", "the answer", |s| s.contains(&selected))?;
    ghtui.keys(", !")?;
    until("the answer marked on GitHub", GITHUB, || {
        Ok((discussion(&id)?["answer"]["id"] == comment_id.as_str()).then_some(()))
    })?;
    ghtui.wait_for("✓ Answered")?;
    ghtui.wait_for(&format!("Answer selected by {}", cx.env.login))?;

    ghtui.wait_followed()?;
    cx.step(",+ reacts to it")?;
    ghtui.select("gg }", "the answer", |s| s.contains(&selected))?;
    ghtui.keys(", +")?;
    ghtui.wait_for("gives or takes back your reaction")?;
    ghtui.select("Down", "🎉", |s| {
        s.lines()
            .any(|l| l.contains("⟦▌") && l.contains("🎉 Hooray"))
    })?;
    ghtui.keys("Enter")?;
    until("the 🎉 on GitHub", GITHUB, || {
        Ok(comments(&id)?
            .iter()
            .find(|c| c["id"] == comment_id.as_str())
            .is_some_and(|c| {
                c["reactions"]["nodes"]
                    .as_array()
                    .is_some_and(|r| r.iter().any(|r| r["content"] == "HOORAY"))
            })
            .then_some(()))
    })?;
    ghtui.wait_for("🎉 1")?;
    Ok(())
}

fn discussion(id: &str) -> Result<Value> {
    let data = gh::graphql(
        "query($id: ID!) { node(id: $id) { ... on Discussion { answer { id }
            comments(first: 20) { nodes { id body
                replies(first: 20) { nodes { body } }
                reactions(first: 20) { nodes { content } } } } } } }",
        json!({"id": id}),
    )?;
    Ok(data["node"].clone())
}

fn comments(id: &str) -> Result<Vec<Value>> {
    Ok(discussion(id)?["comments"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default())
}

fn delete(id: &str) -> Result<()> {
    gh::graphql(
        "mutation($id: ID!) { deleteDiscussion(input: {id: $id}) { clientMutationId } }",
        json!({"id": id}),
    )?;
    Ok(())
}

/// Discussions a killed run left.
fn sweep() -> Result<()> {
    let data = gh::graphql(
        "query($o: String!, $n: String!) { repository(owner: $o, name: $n) {
            discussions(first: 50) { nodes { id title } } } }",
        json!({"o": gh::OWNER, "n": gh::NAME}),
    )?;
    for d in data["repository"]["discussions"]["nodes"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        if d["title"]
            .as_str()
            .is_some_and(|t| t.starts_with("[ghtui live]"))
            && let Some(id) = d["id"].as_str()
        {
            delete(id)?;
        }
    }
    Ok(())
}
