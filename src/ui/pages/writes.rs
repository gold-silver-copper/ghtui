//! The forms for what used to need the browser: a new release, a gist
//! made or changed, a wiki page written.

use super::*;
use crate::ui::page::WriteLink;

/// A row of a form: what it's called, what it's set to, and what Enter on
/// it does (none: it only says).
pub struct FormRow {
    pub name: &'static str,
    pub value: String,
    /// The value is a placeholder or a note, not what's set.
    pub unset: bool,
    pub link: Option<WriteLink>,
}

/// A box of text the composer writes: its title (the first line, or a
/// file's name), and its text.
pub struct TextBox<'a> {
    pub title: String,
    /// What the title says while there's none.
    pub untitled: &'a str,
    pub body: &'a str,
    /// Rendered as Markdown; otherwise as the code it is.
    pub markdown: bool,
    /// What the box says while there's no text.
    pub empty: &'a str,
    pub hint: &'a str,
    /// What Enter on it does.
    pub link: Link,
    /// What else can be done with it, by name ("rename", "remove").
    pub actions: Vec<(&'static str, WriteLink)>,
}

pub struct WriteFormView<'a> {
    pub heading: String,
    /// Where its Markdown's relative links point.
    pub base: Option<LinkBase>,
    pub rows: Vec<FormRow>,
    /// Lines about what's set, as GitHub's notes under a field.
    pub status: Vec<(String, Mark)>,
    pub boxes: Vec<TextBox<'a>>,
    pub after: Vec<FormRow>,
    /// The key that writes the text, and the one that saves.
    pub write: &'a str,
    pub save_key: &'a str,
    pub button: String,
    pub why_not: Option<String>,
    pub error: Option<&'a str>,
}

/// The most lines of a file shown on its form.
const SHOWN: usize = 40;

pub fn write_form(page: &mut Page, v: &WriteFormView<'_>) {
    page.line(vec![Seg::new(v.heading.clone(), Role::Title)]);
    page.blank();
    for row in &v.rows {
        form_row(page, row);
    }
    for (text, mark) in &v.status {
        let (sign, role) = match mark {
            Mark::Fine => ("✓ ", Role::Success),
            Mark::Waiting => ("… ", Role::Meta),
            Mark::Problem => ("! ", Role::Error),
        };
        page.wrapped(
            vec![Seg::new(sign, role), Seg::new(text.clone(), Role::Body)],
            0,
            Frame::None,
        );
    }
    for b in &v.boxes {
        text_box(page, b, v);
    }
    page.blank();
    for row in &v.after {
        form_row(page, row);
    }
    page.blank();
    item(page, Link::Write(WriteLink::Save), |page, link| {
        let role = match v.why_not {
            Some(_) => Role::Chip(Bg::ContainerHigh),
            None => Role::Chip(Bg::SuccessContainer),
        };
        page.line(vec![
            Seg::linked(format!(" {} ", v.button), role, link),
            Seg::new(format!("  {}", v.save_key), Role::Meta),
        ]);
        if let Some(why) = &v.why_not {
            page.wrapped(vec![Seg::new(why.clone(), Role::Meta)], 0, Frame::None);
        }
    });
    if let Some(error) = v.error {
        page.wrapped(
            vec![Seg::new(error.to_owned(), Role::Error)],
            0,
            Frame::None,
        );
    }
}

fn form_row(page: &mut Page, row: &FormRow) {
    let value = |link: Option<u32>| {
        let role = if row.unset { Role::Meta } else { Role::Body };
        match link {
            Some(link) => Seg::linked(row.value.clone(), role, link),
            None => Seg::new(row.value.clone(), role),
        }
    };
    let name = Seg::new(format!("{:<12}", row.name), Role::Strong);
    match &row.link {
        Some(target) => item(page, Link::Write(target.clone()), |page, link| {
            page.wrapped(vec![name, value(Some(link))], 0, Frame::None);
        }),
        None => page.wrapped(vec![name, value(None)], 0, Frame::None),
    }
}

fn text_box(page: &mut Page, b: &TextBox<'_>, v: &WriteFormView<'_>) {
    // A file's name is its own row: Enter renames it, and says so.
    let rename = b.actions.iter().find(|(name, _)| *name == "rename");
    let right = match rename {
        Some(_) => format!("↵ renames · {} writes", v.write),
        None => format!("{} writes", v.write),
    };
    let right = vec![Seg::new(right, Role::Meta)];
    let title_start = page.lines.len();
    let title_link = match rename {
        Some((_, target)) => page.link(Link::Write(target.clone())),
        None => page.link(b.link.clone()),
    };
    let title = if b.title.is_empty() {
        Seg::linked(b.untitled, Role::Meta, title_link)
    } else {
        Seg::linked(b.title.clone(), Role::Strong, title_link)
    };
    page.box_top(vec![title], right);
    if rename.is_some() {
        page.item(title_start, title_link);
    }
    let body_start = page.lines.len();
    let link = page.link(b.link.clone());
    if b.body.trim().is_empty() {
        empty_row(page, b.empty);
    } else if b.markdown {
        markdown::render(page, b.body, v.base.as_ref(), Frame::Body);
    } else {
        let lines: Vec<&str> = b.body.lines().collect();
        for line in lines.iter().take(SHOWN) {
            body(page, vec![Seg::new((*line).to_owned(), Role::Code)]);
        }
        if lines.len() > SHOWN {
            let more = lines.len() - SHOWN;
            empty_row(page, &format!("and {}", plural(more as u64, "more line")));
        }
    }
    page.box_gap();
    body(page, vec![Seg::linked(b.hint.to_owned(), Role::Meta, link)]);
    page.box_bottom();
    page.item(
        if rename.is_some() {
            body_start
        } else {
            title_start
        },
        link,
    );
    for (name, target) in b.actions.iter().filter(|(name, _)| *name != "rename") {
        item(page, Link::Write(target.clone()), |page, link| {
            page.line(vec![Seg::linked(
                format!("  {name} this file"),
                Role::Link,
                link,
            )]);
        });
    }
}
