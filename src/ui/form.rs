//! A form's rows, as its page links them: data only. The form on screen
//! says what Enter on each does ([`crate::forms`]).

/// A row of a form's page.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Row {
    /// Part `n` of the form's text: Enter writes it in the composer.
    Text(usize),
    /// The form's button: makes, saves or runs it.
    Save,
    /// A field, set in a picker (or, for yes-or-no, toggled).
    Field(Field),
    /// A new issue's template, by its file (empty: a blank issue).
    Template(String),
    /// Choose another template.
    Templates,
    Labels,
    Assignees,
    Milestone,
    /// A new pull request's branch to merge into.
    Base,
    /// A new pull request's branch with the changes.
    Head,
    /// Push the head branch.
    Push,
    /// A release's tag.
    Tag,
    /// What a new tag is made at.
    Target,
    /// Have GitHub write a release's notes.
    Notes,
    Prerelease,
    Latest,
    /// A gist's description.
    Description,
    /// Rename the gist's file at this place.
    Rename(usize),
    /// Remove the gist's file at this place.
    Remove(usize),
    AddFile,
    /// The branch a workflow runs on.
    Ref,
    /// A workflow's input.
    Input(String),
    /// The labels page's "New label".
    New,
}

impl Row {
    /// What Enter on it does, for the menu.
    pub fn describe(&self) -> &'static str {
        match self {
            Row::Text(_) => "Write it in the composer",
            Row::Save => "Save it",
            Row::Field(field) => field.describe(),
            Row::Template(file) if file.is_empty() => "Start a blank issue",
            Row::Template(_) => "Start from this template",
            Row::Templates => "Choose another template",
            Row::Labels => "Choose labels",
            Row::Assignees => "Choose assignees",
            Row::Milestone => "Choose a milestone",
            Row::Base => "Choose the branch to merge into",
            Row::Head => "Choose the branch with the changes",
            Row::Push => "Push the branch to GitHub",
            Row::Tag => "Choose a tag, or make one",
            Row::Target => "Choose the branch or commit to tag",
            Row::Notes => "Have GitHub write the release notes",
            Row::Prerelease => "Mark it a pre-release, or not",
            Row::Latest => "Make it the latest release, or not",
            Row::Description => "Write the gist's description",
            Row::Rename(_) => "Rename the file",
            Row::Remove(_) => "Remove the file",
            Row::AddFile => "Add a file",
            Row::Ref => "Choose the branch it runs on",
            Row::Input(_) => "Set the input",
            Row::New => "New label",
        }
    }
}

/// A field of a label's, a milestone's, a fork's, a new repository's or
/// a file's form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Field {
    Name,
    Colour,
    Description,
    Title,
    Due,
    Owner,
    Private,
    Readme,
    Gitignore,
    License,
    AllBranches,
    Path,
    /// A file's commit's branch: empty for the one it's on, else a new one.
    Branch,
    Message,
}

impl Field {
    /// Its row's name.
    pub fn name(self) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Colour => "Colour",
            Field::Description => "Description",
            Field::Title => "Title",
            Field::Due => "Due date",
            Field::Owner => "Owner",
            Field::Private => "Visibility",
            Field::Readme => "README",
            Field::Gitignore => ".gitignore",
            Field::License => "Licence",
            Field::AllBranches => "Branches",
            Field::Path => "Path",
            Field::Branch => "Commit to",
            Field::Message => "Message",
        }
    }

    /// Set by Enter alone: yes or no.
    pub fn toggles(self) -> bool {
        matches!(self, Field::Private | Field::Readme | Field::AllBranches)
    }

    fn describe(self) -> &'static str {
        match self {
            Field::Private => "Make it public or private",
            Field::Readme => "Add a README, or not",
            Field::AllBranches => "Copy all branches, or the default one",
            Field::Owner => "Choose the owner",
            Field::Colour => "Choose the colour",
            Field::Due => "Set the due date",
            Field::Gitignore => "Choose a .gitignore template",
            Field::License => "Choose a licence",
            Field::Path => "Name the file, or move it",
            Field::Branch => "Choose the branch to commit to",
            Field::Message => "Write the commit message",
            Field::Name | Field::Title => "Name it",
            Field::Description => "Describe it",
        }
    }
}
