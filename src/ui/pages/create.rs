//! New issues and pull requests: the template chooser, and a pull
//! request's commits under its form.

use super::*;

/// Something to start a new issue from: a template, a blank issue, or a
/// link the repository sends you to instead.
pub struct Starter {
    pub name: String,
    pub about: String,
    pub link: Link,
}

/// GitHub's template chooser.
pub fn template_chooser(page: &mut Page, repo: &RepoId, starters: &[Starter]) {
    page.line(vec![Seg::new(format!("New issue in {repo}"), Role::Title)]);
    page.line(vec![meta("Choose how to start")]);
    page.box_top(vec![strong("Templates")], Vec::new());
    for (i, s) in starters.iter().enumerate() {
        if i > 0 {
            page.box_rule();
        }
        let external = matches!(&s.link, Link::Url(_));
        item(page, s.link.clone(), |page, link| {
            let open = if external { "Open ↗" } else { "Get started" };
            let right = vec![Seg::new(format!(" {open} "), Role::Chip(Bg::ContainerHigh))];
            link_line(page, s.name.clone(), Role::Strong, link, right);
            if !s.about.trim().is_empty() {
                page.wrapped(vec![meta(s.about.clone())], 0, Frame::Body);
            }
        });
    }
    page.box_bottom();
}

/// A pull request's commits (short ID and subject), and how many more
/// there are, under its form.
pub fn commits_box(page: &mut Page, commits: &[(String, String)], more: u64) {
    let n = commits.len() as u64 + more;
    page.box_top(vec![strong(plural(n, "commit"))], Vec::new());
    if commits.is_empty() {
        empty_row(page, "No commits to merge");
    }
    for (oid, subject) in commits {
        body(
            page,
            vec![
                Seg::new(format!("{oid} "), Role::Code),
                Seg::new(subject.clone(), Role::Body),
            ],
        );
    }
    if more > 0 {
        empty_row(page, &format!("and {}, older", plural(more, "commit")));
    }
    page.box_bottom();
}
