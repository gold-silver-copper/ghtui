//! The composer: text typed for whatever asked for it. That's a review's
//! comment ([`crate::review`]), a comment on a conversation or a commit,
//! an edit of one ([`crate::edit`]), a discussion's ([`crate::discuss`],
//! [`crate::manage`]), a project's draft issue ([`crate::projects`]), a
//! repository's setting ([`crate::settings`]), or a form's text on its
//! page ([`crate::forms`]). Each says the composer's title, what ctrl-s
//! does, and where the text goes.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui_textarea::TextArea;

use crate::act::By;
use crate::api::change::Change;
use crate::api::model::{NodeId, RepoId};
use crate::forms::{Sent, say};
use crate::review::{Comment, EditPurpose, Preview};
use crate::state::{Cmd, Cmds, Overlay, State};
use crate::theme::{Bg, Theme};
use crate::ui::text::short_sha;

/// What the composer's text is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A review's comment, reply or draft.
    Review(Comment),
    /// A comment on an issue or pull request's conversation, posted right
    /// away; quoting the comment made `quoting` (its time, as links name it).
    Conversation {
        subject_id: NodeId,
        name: String,
        quoting: Option<String>,
    },
    /// A comment on a commit as a whole, posted right away.
    Commit { repo: RepoId, oid: String },
    /// Editing a comment or description already on GitHub, saved right
    /// away.
    Edit(crate::edit::Editing),
    /// A comment, a reply or an edit in a discussion, saved right away.
    Discussion(crate::discuss::Writing),
    /// A new discussion: its title on the first line, then its text.
    NewDiscussion(crate::manage::Starting),
    /// A project's draft issue, new or edited: its title on the first
    /// line, then its text.
    ProjectDraft(crate::projects::Drafting),
    /// A repository's rule, webhook or deploy key, as `Name: value` lines.
    Setting(crate::settings::Form),
    /// A form's text, on its page.
    Form(crate::forms::Text),
}

impl Target {
    /// The composer's title.
    pub fn title(&self, state: &State) -> String {
        match self {
            Target::Review(c) => c.title(),
            Target::Conversation { name, .. } => format!("Comment on {name}"),
            Target::Commit { oid, .. } => format!("Comment on commit {}", short_sha(oid)),
            Target::Edit(e) => format!("Edit {}", e.name),
            Target::Discussion(w) => w.title.clone(),
            Target::NewDiscussion(s) => s.title(),
            Target::ProjectDraft(d) => d.title.clone(),
            Target::Setting(f) => f.title(),
            Target::Form(text) => text.title(state),
        }
    }

    /// What ctrl-s does, for the composer's hint.
    pub fn save(&self, state: &State) -> &'static str {
        match self {
            Target::Review(c) => c.save(),
            Target::Conversation { .. } | Target::Commit { .. } => "post comment",
            Target::Edit(_) => "save",
            Target::Discussion(w) => w.save(),
            Target::NewDiscussion(_) => "start discussion",
            Target::ProjectDraft(d) if d.draft.is_some() => "save",
            Target::ProjectDraft(_) => "add draft",
            Target::Setting(f) => f.save(),
            Target::Form(text) => text.save(state),
        }
    }
}

pub struct Compose {
    pub target: Target,
    pub input: TextArea<'static>,
    pub preview: Option<Preview>,
    pub sent: Sent,
    /// The key (esc or ctrl-c) pressed once with text in the editor: only
    /// that key, pressed again, throws the text away.
    pub discard_armed: Option<&'static str>,
    /// The list of who or what an `@` or `#` being typed can be.
    pub completing: crate::typing::Completing,
    /// Your saved replies, listed to insert one (`ctrl-g`).
    pub saved: Option<crate::site::replies::Choosing>,
}

impl Compose {
    pub fn new(theme: &Theme, target: Target, text: &str) -> Self {
        let mut input = editor(theme, text);
        match target {
            // A draft's text goes below its title.
            Target::ProjectDraft(_) if input.lines().len() == 1 => {
                input.insert_newline();
                input.insert_newline();
            }
            // A form starts on its first field: a setting's first value,
            // a discussion's title.
            Target::Setting(_) | Target::NewDiscussion(_) => {
                for to in crate::forms::TITLE {
                    input.move_cursor(*to);
                }
            }
            _ => {}
        }
        Self {
            target,
            input,
            preview: None,
            sent: Sent::default(),
            discard_armed: None,
            completing: crate::typing::Completing::default(),
            saved: None,
        }
    }

    pub fn text(&self) -> String {
        self.input.lines().join("\n")
    }

    /// Why a comment asked for on lines is on the whole file.
    pub fn note(&self) -> Option<&str> {
        match &self.target {
            Target::Review(c) => c.note(),
            _ => None,
        }
    }

    /// Rows the composer wants on screen.
    pub fn rows(&self) -> u16 {
        let preview = (self.preview.as_ref()).map(|p| p.original.len() + p.suggested.len());
        crate::ui::review_sheets::compose_rows(preview, self.note().is_some())
    }
}

/// Why a file isn't attached to a comment: GitHub uploads one only for a
/// signed-in browser, not through its API.
pub(crate) const NO_ATTACHMENTS: &str = "GitHub's API can't attach a file to a comment (only a signed-in browser can upload one), so its path was pasted: upload it on github.com, or link to it";

/// A text box for the composer and the review's summary, holding `text`.
pub(crate) fn editor(theme: &Theme, text: &str) -> TextArea<'static> {
    let mut input = crate::nav::new_input(theme, Bg::ContainerHigh);
    input.set_wrap_mode(ratatui_textarea::WrapMode::WordOrGlyph);
    input.insert_str(text.trim_end_matches('\n'));
    input
}

pub(crate) fn on_key(state: &mut State, key: KeyEvent) -> Cmds {
    let Some(Overlay::Compose(compose)) = &mut state.overlay else {
        return Cmds::new();
    };
    if compose.sent.sending {
        return Cmds::new();
    }
    if let Some(cmds) = crate::site::replies::on_key(state, key) {
        return cmds;
    }
    if let Some(cmds) = crate::typing::on_compose_key(state, key) {
        return cmds;
    }
    let Some(Overlay::Compose(compose)) = &mut state.overlay else {
        return Cmds::new();
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        // A form keeps what's written: its page shows it.
        KeyCode::Esc if let Target::Form(text) = &compose.target => {
            let (text, written) = (text.clone(), compose.text());
            text.keep(state, written);
            state.overlay = None;
        }
        KeyCode::Esc => {
            if may_discard(state, "esc", "discard") {
                state.overlay = None;
            }
        }
        KeyCode::Char('e') if ctrl => {
            return Cmd::Edit {
                purpose: EditPurpose::Compose,
                text: compose.text(),
            }
            .into();
        }
        KeyCode::Char('s') if ctrl => return save(state),
        _ => {
            compose.discard_armed = None;
            compose.sent.error = None;
            compose.completing.selected = 0;
            compose.input.input(key);
        }
    }
    Cmds::new()
}

/// Whether `key` may throw away the text typed in the composer or the
/// review summary: the first press only warns, a second of the same key
/// goes ahead. Each key warns for itself, so esc then ctrl-c still asks.
pub(crate) fn may_discard(state: &mut State, key: &'static str, verb: &str) -> bool {
    let (text, armed, error, what) = match &mut state.overlay {
        Some(Overlay::Compose(c)) => (c.text(), &mut c.discard_armed, &mut c.sent.error, "comment"),
        Some(Overlay::Submit(d)) => (
            d.input.lines().join("\n"),
            &mut d.discard_armed,
            &mut d.sent.error,
            "summary",
        ),
        _ => return true,
    };
    if text.trim().is_empty() || *armed == Some(key) {
        return true;
    }
    *armed = Some(key);
    *error = Some(format!("Press {key} again to {verb} this {what}"));
    false
}

/// `ctrl-s` in the composer: the text goes where its target says.
pub(crate) fn save(state: &mut State) -> Cmds {
    let Some(Overlay::Compose(compose)) = &state.overlay else { return Cmds::new() };
    let (target, body) = (compose.target.clone(), compose.text());
    match target {
        // A form says itself what it lacks.
        Target::Form(text) => text.save_form(state, body),
        _ if body.trim().is_empty() => say(state, "Write something first".into()),
        Target::Review(c) => crate::review::save(state, c, body),
        Target::Conversation { subject_id, .. } => {
            let change = Change::Comment {
                subject: subject_id,
                body,
            };
            posted(state, change, By::Compose)
        }
        Target::Commit { repo, oid } => posted(
            state,
            Change::CommitComment { repo, oid, body },
            By::Compose,
        ),
        Target::Edit(e) => {
            let edit = crate::api::edit::Edit::Body {
                of: e.of,
                id: e.id,
                body,
            };
            posted(state, Change::Edit(edit), By::Edit(e.about))
        }
        Target::Discussion(w) => {
            crate::forms::sending(state);
            crate::discuss::send(&w, body)
        }
        Target::NewDiscussion(s) => crate::manage::start(state, &s, &body),
        Target::ProjectDraft(d) => match d.send(&body) {
            Ok(cmds) => {
                crate::forms::sending(state);
                cmds
            }
            Err(why) => say(state, why),
        },
        Target::Setting(f) => crate::settings::submit(state, &f, &body),
    }
}

/// Sends `change`, the composer waiting for GitHub's answer.
fn posted(state: &mut State, change: Change, by: By) -> Cmds {
    crate::forms::sending(state);
    crate::act::send(change, by).into()
}
