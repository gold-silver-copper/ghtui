//! Older GitHub Enterprise Servers: what they lack of what ghtui asks, so
//! their pages load with what they have.
//!
//! ghtui's queries are written against github.com's schema. An
//! Enterprise Server is a release behind it, or many: a query naming a
//! field it doesn't have fails whole. So ghtui asks the server its
//! version once (`/meta`'s `installed_version`), and before each query
//! leaves out what [`table`] says that version lacks: a field (an
//! answer then reads as GitHub's null, or an empty list, where the page
//! goes without it; or another field stands in, under its name), an
//! argument, a timeline event's fragment and its enum value. What the
//! table doesn't know, the server says when it refuses a query ("Field
//! 'x' doesn't exist on type 'Y'"): ghtui learns it ([`learn`]) and asks
//! again without it. Each thing left out is said once, as what the page
//! goes without. A REST endpoint the server lacks isn't asked at all,
//! and says so ([`rest_lack`]).
//!
//! The table comes from the schemas GitHub publishes for each version
//! (`what_an_enterprise_schema_lacks` in `query_check` lists what one
//! lacks).

use std::fmt;

use serde_json::Value;

/// An Enterprise Server's version: its major and minor (`3.18`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u32, pub u32);

impl Version {
    /// `3.18.2` (or `3.18`) as 3.18.
    pub fn parse(s: &str) -> Option<Version> {
        let mut parts = s.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        Some(Version(major, minor))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.0, self.1)
    }
}

/// What a server is, as ghtui knows it: its version, where it said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Server {
    pub version: Option<Version>,
}

impl Server {
    /// How it's named when something's missing.
    pub fn name(self) -> String {
        match self.version {
            Some(v) => format!("GitHub Enterprise Server {v}"),
            None => "This GitHub Enterprise Server".to_owned(),
        }
    }

    /// Whether it lacks what came in `since` (`None`: no Enterprise
    /// Server has it yet). A server that didn't say its version is taken
    /// to lack it.
    fn lacks(self, since: Option<Version>) -> bool {
        match (self.version, since) {
            (_, None) | (None, _) => true,
            (Some(v), Some(since)) => v < since,
        }
    }
}

/// Something ghtui asks for that an Enterprise Server may not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lack {
    pub missing: Missing,
    /// The first version that has it; `None`: none has it yet.
    pub since: Option<Version>,
    /// What a page goes without, as said once it's left out.
    pub about: String,
}

/// Where in a query it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Missing {
    /// A field by its name, where the selections around it end as one of
    /// `within` says (`items.nodes`: in a field `nodes` in a field
    /// `items`; a name is a field's or a fragment's type; empty:
    /// anywhere), and what stands in for it.
    Field {
        name: String,
        within: Vec<String>,
        fill: Fill,
    },
    /// An argument of a field, by their names.
    Argument { field: String, name: String },
    /// An inline fragment on a type.
    Type(String),
    /// An enum value given in an argument's list.
    EnumValue(String),
}

/// What an answer reads in place of a field left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fill {
    /// GitHub's null.
    Null,
    /// This JSON (an empty list's connection, `false`).
    Json(String),
    /// This other field, asked under the left-out one's name.
    Alias(String),
}

fn v(minor: u32) -> Option<Version> {
    Some(Version(3, minor))
}

fn field(name: &str, within: &[&str], fill: Fill, since: Option<Version>, about: &str) -> Lack {
    Lack {
        missing: Missing::Field {
            name: name.to_owned(),
            within: within.iter().map(|w| (*w).to_owned()).collect(),
            fill,
        },
        since,
        about: about.to_owned(),
    }
}

fn json(text: &str) -> Fill {
    Fill::Json(text.to_owned())
}

fn alias(name: &str) -> Fill {
    Fill::Alias(name.to_owned())
}

/// Event types (their fragments' and their enum values' names) that came
/// in `since`.
fn events(names: &[(&str, &str)], since: Option<Version>, about: &str) -> Vec<Lack> {
    names
        .iter()
        .flat_map(|(ty, value)| {
            [
                Missing::Type((*ty).to_owned()),
                Missing::EnumValue((*value).to_owned()),
            ]
        })
        .map(|missing| Lack {
            missing,
            since,
            about: about.to_owned(),
        })
        .collect()
}

/// What Enterprise Servers lack of what ghtui asks, and since when they
/// have it (from GitHub's published schemas for 3.10 to 3.22).
pub fn table() -> Vec<Lack> {
    let mut lacks = vec![
        field(
            "viewerCanAssign",
            &["pullRequest", "PullRequest"],
            alias("viewerCanUpdate"),
            None,
            "who may assign a pull request (it's offered to whoever may edit it)",
        ),
        field(
            "viewerCanAssign",
            &["issue", "Issue"],
            alias("viewerCanUpdate"),
            v(17),
            "who may assign an issue (it's offered to whoever may edit it)",
        ),
        field(
            "viewerCanSetMilestone",
            &[],
            alias("viewerCanUpdate"),
            None,
            "who may set a milestone (it's offered to whoever may edit)",
        ),
        field(
            "viewerCanLabel",
            &[],
            alias("viewerCanUpdate"),
            v(15),
            "who may label (it's offered to whoever may edit)",
        ),
        field(
            "viewerCanCreateIssues",
            &[],
            alias("hasIssuesEnabled"),
            v(22),
            "who may open an issue (it's offered wherever issues are on)",
        ),
        field("mergeQueueEntry", &[], Fill::Null, v(12), "merge queues"),
        field(
            "isMergeQueueEnabled",
            &[],
            json("false"),
            v(12),
            "merge queues",
        ),
        field(
            "closedByPullRequestsReferences",
            &[],
            Fill::Null,
            v(15),
            "which pull requests will close an issue",
        ),
        field("parent", &["Issue"], Fill::Null, v(16), "sub-issues"),
        field(
            "subIssues",
            &[],
            json(r#"{"totalCount":0,"nodes":[]}"#),
            v(16),
            "sub-issues",
        ),
        field(
            "subIssuesSummary",
            &[],
            json(r#"{"completed":0,"total":0}"#),
            v(16),
            "sub-issues",
        ),
        field("issueType", &[], Fill::Null, v(17), "issue types"),
        field(
            "projectItems",
            &[],
            Fill::Null,
            v(15),
            "the projects an issue or pull request is in",
        ),
        field(
            "projectsV2",
            &["repository"],
            Fill::Null,
            v(15),
            "a repository's projects",
        ),
        field(
            "fullDatabaseId",
            &["items.nodes", "ProjectV2Item"],
            alias("databaseId"),
            v(13),
            "a project item's full ID",
        ),
        field(
            "fullDatabaseId",
            &[
                "PullRequestReview",
                "pullRequestReview",
                "reviews.nodes",
                "latestReviews.nodes",
            ],
            Fill::Null,
            v(13),
            "a review's ID, which links to it",
        ),
        field(
            "isAnswered",
            &[],
            Fill::Null,
            v(12),
            "whether a discussion is answered",
        ),
        field(
            "color",
            &["options"],
            json(r#""GRAY""#),
            v(11),
            "a project field's colors",
        ),
        field(
            "description",
            &["options"],
            json(r#""""#),
            v(11),
            "a project field's descriptions",
        ),
        Lack {
            missing: Missing::Argument {
                field: "items".into(),
                name: "archivedStates".into(),
            },
            since: v(22),
            about: "a project's archived items apart (they're told by each item)".into(),
        },
        Lack {
            missing: Missing::Argument {
                field: "items".into(),
                name: "query".into(),
            },
            since: v(20),
            about: "a project view's filter (all its items are shown)".into(),
        },
    ];
    lacks.extend(events(
        &[
            ("AddedToMergeQueueEvent", "ADDED_TO_MERGE_QUEUE_EVENT"),
            (
                "RemovedFromMergeQueueEvent",
                "REMOVED_FROM_MERGE_QUEUE_EVENT",
            ),
        ],
        v(12),
        "merge queues",
    ));
    lacks.extend(events(
        &[
            ("SubIssueAddedEvent", "SUB_ISSUE_ADDED_EVENT"),
            ("SubIssueRemovedEvent", "SUB_ISSUE_REMOVED_EVENT"),
            ("ParentIssueAddedEvent", "PARENT_ISSUE_ADDED_EVENT"),
            ("ParentIssueRemovedEvent", "PARENT_ISSUE_REMOVED_EVENT"),
        ],
        v(17),
        "sub-issue events in the timeline",
    ));
    lacks.extend(events(
        &[
            ("IssueTypeAddedEvent", "ISSUE_TYPE_ADDED_EVENT"),
            ("IssueTypeChangedEvent", "ISSUE_TYPE_CHANGED_EVENT"),
            ("IssueTypeRemovedEvent", "ISSUE_TYPE_REMOVED_EVENT"),
        ],
        v(18),
        "issue type events in the timeline",
    ));
    lacks.extend(events(
        &[
            ("AddedToProjectV2Event", "ADDED_TO_PROJECT_V2_EVENT"),
            ("RemovedFromProjectV2Event", "REMOVED_FROM_PROJECT_V2_EVENT"),
            (
                "ProjectV2ItemStatusChangedEvent",
                "PROJECT_V2_ITEM_STATUS_CHANGED_EVENT",
            ),
        ],
        v(20),
        "project events in the timeline",
    ));
    lacks
}

/// What `server` lacks of [`table`].
pub fn lacks(server: Server) -> Vec<Lack> {
    table()
        .into_iter()
        .filter(|l| server.lacks(l.since))
        .collect()
}

/// What a server's refusal of a query says it lacks: GitHub's
/// "Field 'x' doesn't exist on type 'Y'", "Field 'x' doesn't accept
/// argument 'a'", and "No such type T, so it can't be a fragment
/// condition".
pub fn learn(message: &str) -> Option<Lack> {
    // The names in quotes (not "doesn't"'s apostrophe).
    let quoted = |s: &str| -> Vec<String> {
        let mut names = Vec::new();
        let mut rest = s;
        while let Some(start) = rest.find('\'') {
            rest = rest.get(start + 1..).unwrap_or_default();
            let word = word_at(rest, 0);
            if !word.is_empty() && rest.get(word.len()..).is_some_and(|r| r.starts_with('\'')) {
                names.push(word.to_owned());
                rest = rest.get(word.len() + 1..).unwrap_or_default();
            }
        }
        names
    };
    let missing = if message.contains("doesn't exist on type") {
        let names = quoted(message);
        Missing::Field {
            name: names.first()?.clone(),
            within: Vec::new(),
            fill: Fill::Null,
        }
    } else if message.contains("doesn't accept argument") {
        let names = quoted(message);
        Missing::Argument {
            field: names.first()?.clone(),
            name: names.get(1)?.clone(),
        }
    } else {
        let rest = message.strip_prefix("No such type ")?;
        Missing::Type(rest.split([',', ' ']).next()?.to_owned())
    };
    let about = match &missing {
        Missing::Field { name, .. } | Missing::Type(name) => format!("`{name}`"),
        Missing::Argument { field, name } => format!("`{field}({name}:)`"),
        Missing::EnumValue(value) => format!("`{value}`"),
    };
    Some(Lack {
        missing,
        since: None,
        about,
    })
}

// ---- REST ---------------------------------------------------------------------

/// A REST endpoint an Enterprise Server may not have: its path, segment
/// by segment (`*` any one), and since when it does.
struct RestLack {
    path: &'static [&'static str],
    since: Option<Version>,
    about: &'static str,
}

const REST: &[RestLack] = &[RestLack {
    path: &["repos", "*", "*", "security-advisories"],
    since: None,
    about: "a repository's security advisories",
}];

/// What `server` lacks for REST path `path` (with its query), if it lacks
/// it.
pub fn rest_lack(server: Server, path: &str) -> Option<&'static str> {
    let path = path.split('?').next().unwrap_or(path);
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    REST.iter()
        .find(|l| {
            server.lacks(l.since)
                && segments.len() >= l.path.len()
                && l.path
                    .iter()
                    .zip(&segments)
                    .all(|(want, got)| *want == "*" || want == got)
        })
        .map(|l| l.about)
}

// ---- a query, without what's missing ------------------------------------------------

/// A query rewritten without what's missing: its text, what each answer
/// reads in place of a field left out (by its path of answer keys), the
/// variables no longer used, and what was left out.
#[derive(Debug, Clone, PartialEq)]
pub struct Rewritten {
    pub query: String,
    pub fills: Vec<(Vec<String>, Value)>,
    pub unused: Vec<String>,
    pub about: Vec<String>,
}

/// `query` without what `lacks` names, or `None` if it names none of it
/// (or doesn't parse, when it's sent as it is).
pub fn rewrite(query: &str, lacks: &[Lack]) -> Option<Rewritten> {
    let mut doc = document(query)?;
    let mut edit = Edit {
        lacks,
        fills: Vec::new(),
        about: Vec::new(),
    };
    for def in &mut doc {
        let (set, path) = match def {
            Def::Op { set, .. } => (set, Some(Vec::new())),
            Def::Fragment { set, .. } => (set, None),
        };
        edit.selections(set, &[], path.as_ref());
    }
    if edit.about.is_empty() {
        return None;
    }
    let mut body = String::new();
    for def in &doc {
        if let Def::Fragment { .. } = def {
            print_def(def, &[], &mut body);
        }
        if let Def::Op { set, .. } = def {
            print_set(set, &mut body);
        }
    }
    // Variables only a left-out argument used are left out too.
    let used = |name: &str| {
        body.match_indices('$')
            .any(|(i, _)| word_at(&body, i + 1) == name)
    };
    let mut unused = Vec::new();
    let mut out = String::new();
    for def in &doc {
        if let Def::Op { vars, .. } = def {
            unused.extend(
                vars.iter()
                    .filter(|v| !used(&v.name))
                    .map(|v| v.name.clone()),
            );
        }
        if !out.is_empty() {
            out.push(' ');
        }
        print_def(def, &unused, &mut out);
    }
    edit.about.dedup();
    Some(Rewritten {
        query: out,
        fills: edit.fills,
        unused,
        about: edit.about,
    })
}

/// The name starting at byte `at` of `s`.
fn word_at(s: &str, at: usize) -> &str {
    let rest = s.get(at..).unwrap_or_default();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    rest.get(..end).unwrap_or_default()
}

/// Puts each fill in `data` where its path leads (through lists, into
/// each item), where the answer doesn't say it already.
pub fn patch(data: &mut Value, fills: &[(Vec<String>, Value)]) {
    for (path, value) in fills {
        fill(data, path, value);
    }
}

fn fill(at: &mut Value, path: &[String], value: &Value) {
    match at {
        Value::Array(items) => {
            for item in items {
                fill(item, path, value);
            }
        }
        Value::Object(map) => match path {
            [] => {}
            [last] => {
                map.entry(last.clone()).or_insert_with(|| value.clone());
            }
            [first, rest @ ..] => {
                if let Some(next) = map.get_mut(first) {
                    fill(next, rest, value);
                }
            }
        },
        _ => {}
    }
}

struct Edit<'a> {
    lacks: &'a [Lack],
    fills: Vec<(Vec<String>, Value)>,
    about: Vec<String>,
}

impl Edit<'_> {
    fn note(&mut self, lack: &Lack) {
        if !self.about.contains(&lack.about) {
            self.about.push(lack.about.clone());
        }
    }

    /// `set`, without what's missing; `around` are the selections around
    /// it, outermost first, `path` its answer's keys (`None` in a named
    /// fragment, whose place in answers isn't known).
    fn selections(&mut self, set: &mut Vec<Sel>, around: &[String], path: Option<&Vec<String>>) {
        let inside = |within: &str| {
            let names: Vec<&str> = within.split('.').collect();
            around.len() >= names.len()
                && (around.iter().rev())
                    .zip(names.iter().rev())
                    .all(|(a, n)| a == n)
        };
        let mut kept = Vec::new();
        for mut sel in std::mem::take(set) {
            match &mut sel {
                Sel::Field(f) => {
                    let key = f.alias.clone().unwrap_or_else(|| f.name.clone());
                    let here = path.map(|p| {
                        let mut p = p.clone();
                        p.push(key.clone());
                        p
                    });
                    let lack = self.lacks.iter().find(|l| match &l.missing {
                        Missing::Field { name, within, .. } => {
                            *name == f.name
                                && (within.is_empty() || within.iter().any(|w| inside(w)))
                        }
                        _ => false,
                    });
                    if let Some(lack) = lack {
                        self.note(lack);
                        let Missing::Field { fill, .. } = &lack.missing else {
                            continue;
                        };
                        match fill {
                            Fill::Alias(other) => kept.push(Sel::Field(Field {
                                alias: Some(key),
                                name: other.clone(),
                                args: Vec::new(),
                                directives: String::new(),
                                set: None,
                            })),
                            Fill::Null => {
                                if let Some(here) = here {
                                    self.fills.push((here, Value::Null));
                                }
                            }
                            Fill::Json(text) => {
                                let value = serde_json::from_str(text).unwrap_or(Value::Null);
                                if let Some(here) = here {
                                    self.fills.push((here, value));
                                }
                            }
                        }
                        continue;
                    }
                    let name = f.name.clone();
                    let mut args = Vec::new();
                    for (arg, mut value) in std::mem::take(&mut f.args) {
                        let gone = self.lacks.iter().find(|l| {
                            matches!(&l.missing, Missing::Argument { field, name: a }
                                if *field == name && *a == arg)
                        });
                        match gone {
                            Some(lack) => self.note(lack),
                            None => {
                                self.values(&mut value);
                                args.push((arg, value));
                            }
                        }
                    }
                    f.args = args;
                    if let Some(inner) = &mut f.set {
                        let mut deeper = around.to_vec();
                        deeper.push(name);
                        self.selections(inner, &deeper, here.as_ref());
                        if inner.is_empty() {
                            inner.push(Sel::Field(Field::named("__typename")));
                        }
                    }
                    kept.push(sel);
                }
                Sel::Inline { on, set: inner, .. } => {
                    if let Some(ty) = on.clone() {
                        let lack = self
                            .lacks
                            .iter()
                            .find(|l| matches!(&l.missing, Missing::Type(t) if *t == ty));
                        if let Some(lack) = lack {
                            self.note(lack);
                            continue;
                        }
                    }
                    let mut deeper = around.to_vec();
                    deeper.extend(on.clone());
                    self.selections(inner, &deeper, path);
                    if inner.is_empty() {
                        continue;
                    }
                    kept.push(sel);
                }
                Sel::Spread(_) => kept.push(sel),
            }
        }
        *set = kept;
    }

    /// An argument's value, without the enum values that are missing.
    fn values(&mut self, value: &mut Val) {
        match value {
            Val::List(items) => {
                let mut kept = Vec::new();
                for mut item in std::mem::take(items) {
                    if let Val::Enum(name) = &item {
                        let lack = self
                            .lacks
                            .iter()
                            .find(|l| matches!(&l.missing, Missing::EnumValue(e) if e == name));
                        if let Some(lack) = lack {
                            self.note(lack);
                            continue;
                        }
                    }
                    self.values(&mut item);
                    kept.push(item);
                }
                *items = kept;
            }
            Val::Object(fields) => {
                for (_, v) in fields {
                    self.values(v);
                }
            }
            _ => {}
        }
    }
}

// ---- reading and writing GraphQL's documents ---------------------------------------

/// A query's definitions, read by cynic's parser into what [`Edit`]
/// changes and the printer writes; scalars, variables and types are kept
/// as written.
fn document(query: &str) -> Option<Vec<Def>> {
    use cynic_parser::executable::ExecutableDefinition;
    let doc = cynic_parser::parse_executable_document(query).ok()?;
    let text = |span: cynic_parser::Span| query.get(span.start..span.end).unwrap_or_default();
    let defs = doc.definitions().map(|def| match def {
        ExecutableDefinition::Operation(op) => {
            let mut head = Vec::new();
            if op.operation_type_span().is_some() {
                head.push(op.operation_type().as_str().to_owned());
            }
            head.extend(op.name().map(str::to_owned));
            let vars = (op.variable_definitions())
                .map(|v| {
                    let mut rest = text(v.ty().span()).to_owned();
                    if let Some(default) = v.default_value() {
                        rest.push_str(" = ");
                        rest.push_str(text(default.span()));
                    }
                    rest.push_str(&directives(v.directives(), &text));
                    VarDef {
                        name: v.name().to_owned(),
                        rest,
                    }
                })
                .collect();
            Def::Op {
                head,
                vars,
                directives: directives(op.directives(), &text),
                set: selections(op.selection_set(), &text),
            }
        }
        ExecutableDefinition::Fragment(f) => Def::Fragment {
            head: ["fragment", f.name(), "on", f.type_condition()]
                .map(str::to_owned)
                .to_vec(),
            set: selections(f.selection_set(), &text),
        },
    });
    Some(defs.collect())
}

fn selections<'a>(
    set: impl Iterator<Item = cynic_parser::executable::Selection<'a>>,
    text: &impl Fn(cynic_parser::Span) -> &'a str,
) -> Vec<Sel> {
    use cynic_parser::executable::Selection;
    set.map(|sel| match sel {
        Selection::Field(f) => Sel::Field(Field {
            alias: f.alias().map(str::to_owned),
            name: f.name().to_owned(),
            args: arguments(f.arguments(), text),
            directives: directives(f.directives(), text),
            set: f
                .selection_set_span()
                .map(|_| selections(f.selection_set(), text)),
        }),
        Selection::InlineFragment(i) => Sel::Inline {
            on: i.type_condition().map(str::to_owned),
            directives: directives(i.directives(), text),
            set: selections(i.selection_set(), text),
        },
        Selection::FragmentSpread(s) => Sel::Spread(s.fragment_name().to_owned()),
    })
    .collect()
}

fn arguments<'a>(
    args: impl Iterator<Item = cynic_parser::executable::Argument<'a>>,
    text: &impl Fn(cynic_parser::Span) -> &'a str,
) -> Vec<(String, Val)> {
    args.map(|a| (a.name().to_owned(), value(a.value(), text)))
        .collect()
}

/// Directives, as printed.
fn directives<'a>(
    dirs: impl Iterator<Item = cynic_parser::executable::Directive<'a>>,
    text: &impl Fn(cynic_parser::Span) -> &'a str,
) -> String {
    dirs.map(|d| {
        let args = print_args(&arguments(d.arguments(), text));
        format!(" @{}{args}", d.name())
    })
    .collect()
}

fn value<'a>(v: cynic_parser::Value<'a>, text: &impl Fn(cynic_parser::Span) -> &'a str) -> Val {
    use cynic_parser::Value;
    match v {
        Value::Enum(e) => Val::Enum(e.name().to_owned()),
        Value::List(l) => Val::List(l.items().map(|i| value(i, text)).collect()),
        Value::Object(o) => Val::Object(
            (o.fields())
                .map(|f| (f.name().to_owned(), value(f.value(), text)))
                .collect(),
        ),
        v => Val::Raw(text(v.span()).to_owned()),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Def {
    /// `query Name(…) @dir { … }`: its words before the variables, the
    /// variables, its directives as written, and its selections.
    Op {
        head: Vec<String>,
        vars: Vec<VarDef>,
        directives: String,
        set: Vec<Sel>,
    },
    /// `fragment Name on Type { … }`, its words before the selections.
    Fragment { head: Vec<String>, set: Vec<Sel> },
}

#[derive(Debug, Clone, PartialEq)]
struct VarDef {
    name: String,
    /// Its type and default, as written.
    rest: String,
}

#[derive(Debug, Clone, PartialEq)]
enum Sel {
    Field(Field),
    Inline {
        on: Option<String>,
        directives: String,
        set: Vec<Sel>,
    },
    Spread(String),
}

#[derive(Debug, Clone, PartialEq)]
struct Field {
    alias: Option<String>,
    name: String,
    args: Vec<(String, Val)>,
    directives: String,
    set: Option<Vec<Sel>>,
}

impl Field {
    fn named(name: &str) -> Field {
        Field {
            alias: None,
            name: name.to_owned(),
            args: Vec::new(),
            directives: String::new(),
            set: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Val {
    Enum(String),
    List(Vec<Val>),
    Object(Vec<(String, Val)>),
    /// A variable, string, number, boolean or null, as written.
    Raw(String),
}

fn print_val(v: &Val, out: &mut String) {
    match v {
        Val::Enum(s) | Val::Raw(s) => out.push_str(s),
        Val::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                print_val(item, out);
            }
            out.push(']');
        }
        Val::Object(fields) => {
            out.push('{');
            for (i, (name, value)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(name);
                out.push_str(": ");
                print_val(value, out);
            }
            out.push('}');
        }
    }
}

fn print_args(args: &[(String, Val)]) -> String {
    if args.is_empty() {
        return String::new();
    }
    let mut out = String::from("(");
    for (i, (name, value)) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(name);
        out.push_str(": ");
        print_val(value, &mut out);
    }
    out.push(')');
    out
}

fn print_set(set: &[Sel], out: &mut String) {
    out.push('{');
    for sel in set {
        out.push(' ');
        match sel {
            Sel::Field(f) => {
                if let Some(alias) = &f.alias {
                    out.push_str(alias);
                    out.push_str(": ");
                }
                out.push_str(&f.name);
                out.push_str(&print_args(&f.args));
                out.push_str(&f.directives);
                if let Some(inner) = &f.set {
                    out.push(' ');
                    print_set(inner, out);
                }
            }
            Sel::Inline {
                on,
                directives,
                set,
            } => {
                out.push_str("...");
                if let Some(on) = on {
                    out.push_str(" on ");
                    out.push_str(on);
                }
                out.push_str(directives);
                out.push(' ');
                print_set(set, out);
            }
            Sel::Spread(name) => {
                out.push_str("...");
                out.push_str(name);
            }
        }
    }
    out.push_str(" }");
}

fn print_def(def: &Def, unused: &[String], out: &mut String) {
    match def {
        Def::Op {
            head,
            vars,
            directives,
            set,
        } => {
            out.push_str(&head.join(" "));
            let vars: Vec<String> = (vars.iter())
                .filter(|v| !unused.contains(&v.name))
                .map(|v| format!("${}: {}", v.name, v.rest))
                .collect();
            if !vars.is_empty() {
                out.push('(');
                out.push_str(&vars.join(", "));
                out.push(')');
            }
            out.push_str(directives);
            if !out.is_empty() && !out.ends_with(' ') {
                out.push(' ');
            }
            print_set(set, out);
        }
        Def::Fragment { head, set } => {
            out.push_str(&head.join(" "));
            out.push(' ');
            print_set(set, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(minor: u32) -> Server {
        Server {
            version: Some(Version(3, minor)),
        }
    }

    /// An old server's query leaves out what it lacks: a field (its
    /// answer reads null there), a field another stands in for, an
    /// argument and the variable only it used, a fragment and its enum
    /// value; the rest is as it was.
    #[test]
    fn a_query_leaves_out_what_the_server_lacks() {
        let query = r#"query Q($owner: String!, $archived: [ProjectV2ItemArchivedState!]) {
            repository(owner: $owner, name: "r") {
              pullRequest(number: 1) { id viewerCanAssign mergeQueueEntry { position } title }
              issueOrPullRequest(number: 2) { ... on Issue { subIssues(first: 50) { totalCount nodes { id } } } }
              timelineItems(last: 100, itemTypes: [ISSUE_COMMENT, ADDED_TO_PROJECT_V2_EVENT]) {
                nodes { __typename ... on AddedToProjectV2Event { id } ... on IssueComment { body } }
              }
            }
            owner: repositoryOwner(login: "o") { ... on ProjectV2Owner { projectV2(number: 1) {
              items(first: 50, archivedStates: $archived) { totalCount } } } }
          }"#;
        let done = rewrite(query, &lacks(server(19))).unwrap();
        assert_eq!(
            done.query,
            "query Q($owner: String!) { repository(owner: $owner, name: \"r\") { pullRequest(number: 1) { id viewerCanAssign: viewerCanUpdate mergeQueueEntry { position } title } issueOrPullRequest(number: 2) { ... on Issue { subIssues(first: 50) { totalCount nodes { id } } } } timelineItems(last: 100, itemTypes: [ISSUE_COMMENT]) { nodes { __typename ... on IssueComment { body } } } } owner: repositoryOwner(login: \"o\") { ... on ProjectV2Owner { projectV2(number: 1) { items(first: 50) { totalCount } } } } }"
        );
        assert_eq!(done.unused, ["archived"]);
        // 3.11 lacks merge queues and sub-issues too.
        let old = rewrite(query, &lacks(server(11))).unwrap();
        assert!(!old.query.contains("mergeQueueEntry") && !old.query.contains("subIssues"));
        let mut data = serde_json::json!({"repository": {"pullRequest": {"id": "PR_1"},
            "issueOrPullRequest": {"__typename": "Issue"}}});
        patch(&mut data, &old.fills);
        assert_eq!(
            data["repository"]["pullRequest"]["mergeQueueEntry"],
            Value::Null
        );
        assert_eq!(
            data["repository"]["issueOrPullRequest"]["subIssues"],
            serde_json::json!({"totalCount": 0, "nodes": []})
        );
        assert!(old.about.contains(&"merge queues".to_owned()));
        // github.com's own version, or a new enough one, lacks nothing.
        assert!(rewrite("query { viewer { login } }", &lacks(server(22))).is_none());
    }

    /// What a server says it lacks, when it refuses a query, is learned.
    #[test]
    fn a_refusal_says_what_the_server_lacks() {
        let field = learn("Field 'mergeQueueEntry' doesn't exist on type 'PullRequest'").unwrap();
        assert!(
            matches!(field.missing, Missing::Field { ref name, .. } if name == "mergeQueueEntry")
        );
        let arg = learn("Field 'items' doesn't accept argument 'archivedStates'").unwrap();
        assert_eq!(
            arg.missing,
            Missing::Argument {
                field: "items".into(),
                name: "archivedStates".into()
            }
        );
        let ty = learn("No such type SubIssueAddedEvent, so it can't be a fragment condition");
        assert_eq!(
            ty.map(|l| l.missing),
            Some(Missing::Type("SubIssueAddedEvent".into()))
        );
        assert_eq!(learn("Something went wrong"), None);
        let rewritten = rewrite(
            "query { repository { pullRequest { mergeQueueEntry { position } } } }",
            &[field],
        )
        .unwrap();
        assert_eq!(
            rewritten.query,
            "query { repository { pullRequest { __typename } } }"
        );
    }

    #[test]
    fn versions_and_rest_paths() {
        assert_eq!(Version::parse("3.18.2"), Some(Version(3, 18)));
        assert_eq!(Version::parse("3.9"), Some(Version(3, 9)));
        assert!(Version(3, 9) < Version(3, 18));
        let path = "/repos/o/r/security-advisories?state=published&per_page=30";
        assert!(rest_lack(server(22), path).is_some());
        assert!(rest_lack(server(22), "/repos/o/r/releases").is_none());
    }
}
