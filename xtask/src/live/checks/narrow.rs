//! At 90 columns, text that doesn't fit ends in `…` rather than being
//! cut off: the header's crumbs, a file's path, the palette's placeholder.

use anyhow::Result;

use crate::live::{Check, Cx, Launch, NARROW, gh};

pub const CHECK: Check = Check::new(
    "narrow",
    "at 90 columns, cut text ends in …: the header's crumbs, a file's path, the palette's placeholder",
    run,
);

fn run(cx: &mut Cx) -> Result<()> {
    let file = format!(
        "https://github.com/{}/blob/main/.github/workflows/botreview.yml",
        gh::REPO
    );
    let ghtui = cx.launch(Launch::new(&[&file]).size(NARROW))?;
    ghtui.wait_for("name: botreview")?;
    cx.step("the header's repository and the file's path")?;
    ghtui.expect_whole_or_cut("gold-silver-copper / ghtui-test-writes")?;
    ghtui.expect_whole_or_cut("ghtui-test-writes / .github / workflows / botreview.yml")?;
    cx.step("the command palette's placeholder")?;
    ghtui.keys(":")?;
    ghtui.wait_for("A command")?;
    ghtui
        .expect_whole_or_cut("A command, owner/repo, owner/repo#123, @user, a URL, or a search")?;
    ghtui.keys("Escape")?;
    Ok(())
}
