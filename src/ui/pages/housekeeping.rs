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
    let base = url::repo(repo);
    let labels = link_seg(page, "Labels", format!("{base}/labels"), Role::Link);
    let milestones = link_seg(page, "Milestones", format!("{base}/milestones"), Role::Link);
    page.wrapped(vec![labels, meta(" · "), milestones], 0, Frame::None);
}

/// A repository's labels by name: each one's chip, description, and its
/// open issues and pull requests, which its row opens.
pub fn labels_page(
    page: &mut Page,
    repo: &RepoId,
    list: Fetched<'_, Results<LabelInfo>>,
    keys: &LabelKeys,
) {
    let new = button(page, "New label", Link::Form(Row::New));
    let (n, edit, delete) = (&keys.new, &keys.edit, &keys.delete);
    let how = format!("  {n} · {edit} edits the selected label · {delete} deletes it");
    page.wrapped(vec![new, meta(how)], 0, Frame::None);
    page.blank();
    let Some(l) = list.show(page, "labels") else { return };
    let none = "No labels yet.";
    counted_box(page, "Labels", Vec::new(), l, none, |page, label| {
        item(page, url::label(repo, &label.name), |page, link| {
            let (name, color) = (format!(" {} ", label.name), label.color.clone());
            let chip = Seg::linked(name, Role::Label(color), link);
            let open = match label.open {
                0 => String::new(),
                n => format!("{} open", compact(n)),
            };
            page.box_line(vec![chip], vec![meta(open)], 0);
            if !label.description.is_empty() {
                body(page, vec![meta(label.description.clone())]);
            }
        });
    });
}
