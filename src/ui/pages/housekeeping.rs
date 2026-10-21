//! A repository's labels page, and the way to it from the Issues and Pull
//! requests tabs ([`crate::housekeeping`] changes them).

use super::*;
use crate::api::housekeeping::LabelInfo;
use crate::ui::form::Row;

/// The keys the labels page names.
pub struct LabelKeys {
    pub new: String,
    pub edit: String,
    pub delete: String,
}

/// `Labels · Milestones`, above an issue or pull request list, as
/// GitHub's buttons beside its filter.
pub fn labels_and_milestones(page: &mut Page, repo: &RepoId) {
    let labels = link_seg(
        page,
        "Labels",
        format!("{}/labels", url::repo(repo)),
        Role::Link,
    );
    let milestones = link_seg(
        page,
        "Milestones",
        format!("{}/milestones", url::repo(repo)),
        Role::Link,
    );
    page.wrapped(vec![labels, meta(" · "), milestones], 0, Frame::None);
}

/// A repository's labels by name: each one's chip, description, and its
/// open issues and pull requests, which its row opens.
pub fn labels_page(
    page: &mut Page,
    repo: &RepoId,
    list: Fetched<'_, Results<LabelInfo>>,
    keys: &LabelKeys,
    _now: u64,
) {
    let new = button(page, "New label", Link::Form(Row::New));
    page.wrapped(
        vec![
            new,
            meta(format!(
                "  {} · {} edits the selected label · {} deletes it",
                keys.new, keys.edit, keys.delete
            )),
        ],
        0,
        Frame::None,
    );
    page.blank();
    let Some(l) = list.show(page, "labels") else {
        return;
    };
    let title = counted("Labels", l);
    list_box(
        page,
        title,
        Vec::new(),
        l,
        "No labels yet.",
        |page, label| {
            let target = url::label(repo, &label.name);
            item(page, target, |page, link| {
                let chip = Seg::linked(
                    format!(" {} ", label.name),
                    Role::Label(label.color.clone()),
                    link,
                );
                let open = match label.open {
                    0 => String::new(),
                    n => format!("{} open", compact(n)),
                };
                let right = vec![meta(open)];
                page.box_line(vec![chip], right, 0);
                if !label.description.is_empty() {
                    body(page, vec![meta(label.description.clone())]);
                }
            });
        },
    );
}
