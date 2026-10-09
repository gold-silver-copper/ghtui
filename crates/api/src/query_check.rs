//! Every GraphQL query ghtui sends, checked against GitHub's schema and
//! for lists cut short without saying so.
//!
//! The typed (cynic) queries are checked against the schema when they
//! compile, the raw ones ([`crate::raw`]) only here. Both are checked here
//! that:
//! - every field with `first:` or `last:` either pages (`pageInfo`) or
//!   fetches `totalCount`, so the page can say what it left out;
//! - no `last:` comes with a descending `orderBy` (that's the oldest N,
//!   oldest last, not the newest);
//! - no list asks for more than 100, and the query's worst-case node
//!   count stays under GitHub's limit of 500,000.

use std::collections::{HashMap, HashSet};

use cynic_parser::executable::{ExecutableDefinition, Iter, Selection};
use cynic_parser::type_system::{Definition, TypeDefinition};
use cynic_parser::{ExecutableDocument, Value};

/// Lists that are fine without a total, and why. (A list of one, `first:
/// 1`, is a "the newest" or "the first", not a list, and needs none.)
const ALLOWED: &[(&str, &str)] = &[
    (
        "q::LastReviewQuery: repository.pullRequest.reviews",
        "Your newest submitted review: you have at most one pending review on a PR, so your last 20 include it",
    ),
    ("b::ProfileQuery: user.pinnedItems", "GitHub pins at most 6"),
    (
        "b::ProfileQuery: organization.pinnedItems",
        "GitHub pins at most 6",
    ),
    (
        "b::ProfileQuery: user.socialAccounts",
        "GitHub keeps at most 4 social accounts",
    ),
    (
        "BRANCHES: repository.refs.nodes.associatedPullRequests",
        "Only the newest from this branch in this repository is shown; 10 newer ones from forks' branches of the same name is not worth paging for",
    ),
];

/// A list's size when its `first:` is a variable: GitHub's most.
const VARIABLE_FIRST: u64 = 100;

/// GitHub's limit on the nodes one query may ask for.
const NODE_LIMIT: u64 = 500_000;

struct Field<'a> {
    ty: &'a str,
    args: Vec<&'a str>,
    /// Its `orderBy` is descending unless a query says otherwise.
    descending: bool,
}

enum Type<'a> {
    /// An object or interface.
    Fields(HashMap<&'a str, Field<'a>>),
    Union,
    Leaf,
}

struct Schema<'a>(HashMap<&'a str, Type<'a>>);

impl<'a> Schema<'a> {
    fn new(doc: &'a cynic_parser::TypeSystemDocument, text: &'a str) -> Self {
        let mut types = HashMap::new();
        for def in doc.definitions() {
            let Definition::Type(ty) = def else { continue };
            let fields = |fields: cynic_parser::type_system::Iter<'a, _>| {
                Type::Fields(
                    fields
                        .map(|f: cynic_parser::type_system::FieldDefinition<'a>| {
                            let descending = f.arguments().any(|a| {
                                let span = a.default_value_span();
                                a.name() == "orderBy"
                                    && text
                                        .get(span.start..span.end)
                                        .is_some_and(|d| d.contains("DESC"))
                            });
                            let field = Field {
                                ty: f.ty().name(),
                                args: f.arguments().map(|a| a.name()).collect(),
                                descending,
                            };
                            (f.name(), field)
                        })
                        .collect(),
                )
            };
            let entry = match ty {
                TypeDefinition::Object(o) => fields(o.fields()),
                TypeDefinition::Interface(i) => fields(i.fields()),
                TypeDefinition::Union(_) => Type::Union,
                _ => Type::Leaf,
            };
            types.insert(ty.name(), entry);
        }
        Self(types)
    }
}

/// What checking one query found.
#[derive(Default)]
struct Report {
    errors: Vec<String>,
    nodes: u64,
    /// The allowed exceptions this query needed.
    allowed: Vec<String>,
}

struct Walk<'a, 'd> {
    schema: &'a Schema<'a>,
    doc: &'d ExecutableDocument,
    name: &'a str,
    report: Report,
}

/// A list argument's size, if it's one.
fn size(value: Value<'_>) -> Option<u64> {
    match value {
        Value::Int(i) => u64::try_from(i.as_i64()).ok(),
        Value::Variable(_) => Some(VARIABLE_FIRST),
        _ => None,
    }
}

impl<'d> Walk<'_, 'd> {
    fn selections(&mut self, ty: &str, set: Iter<'d, Selection<'d>>, path: &str, above: u64) {
        for selection in set {
            match selection {
                Selection::Field(field) => self.field(ty, field, path, above),
                Selection::InlineFragment(fragment) => {
                    let on = fragment.type_condition().unwrap_or(ty);
                    if !self.schema.0.contains_key(on) {
                        self.error(&format!("{path}: no type {on}"));
                    }
                    self.selections(on, fragment.selection_set(), path, above);
                }
                Selection::FragmentSpread(spread) => {
                    let found = self.doc.definitions().find_map(|d| match d {
                        ExecutableDefinition::Fragment(f) if f.name() == spread.fragment_name() => {
                            Some(f)
                        }
                        _ => None,
                    });
                    match found {
                        Some(f) => {
                            self.selections(f.type_condition(), f.selection_set(), path, above);
                        }
                        None => {
                            self.error(&format!("{path}: no fragment {}", spread.fragment_name()));
                        }
                    }
                }
            }
        }
    }

    fn field(
        &mut self,
        ty: &str,
        field: cynic_parser::executable::FieldSelection<'d>,
        path: &str,
        above: u64,
    ) {
        let name = field.name();
        if name == "__typename" {
            return;
        }
        let shown = field.alias().unwrap_or(name);
        let path = if path.is_empty() {
            shown.to_owned()
        } else {
            format!("{path}.{shown}")
        };
        let Some(Type::Fields(fields)) = self.schema.0.get(ty) else {
            return self.error(&format!("{path}: {ty} has no fields"));
        };
        let Some(def) = fields.get(name) else {
            return self.error(&format!("{path}: no field {name} on {ty}"));
        };
        let mut count = None;
        let mut last = false;
        let mut descending = def.descending;
        for arg in field.arguments() {
            if !def.args.contains(&arg.name()) {
                self.error(&format!("{path}: {ty}.{name} takes no {}", arg.name()));
            }
            match arg.name() {
                "first" | "last" => {
                    last |= arg.name() == "last";
                    count = size(arg.value());
                    if count.is_none_or(|n| n > 100) {
                        self.error(&format!("{path}: asks for more than 100"));
                    }
                }
                "orderBy" => {
                    descending = match arg.value() {
                        Value::Object(order) => order
                            .get("direction")
                            .is_some_and(|d| matches!(d, Value::Enum(e) if e.name() == "DESC")),
                        // A variable's order isn't known here.
                        _ => false,
                    };
                }
                _ => {}
            }
        }
        let mut here = above;
        if let Some(n) = count {
            here = above.saturating_mul(n.max(1));
            self.report.nodes = self.report.nodes.saturating_add(here);
            let selected: HashSet<&str> = field
                .selection_set()
                .filter_map(|s| s.as_field().map(|f| f.name()))
                .collect();
            // A search's total is its `issueCount`, `repositoryCount`, ….
            let told = selected.contains("pageInfo")
                || selected
                    .iter()
                    .any(|f| f.ends_with("Count") && *f != "upvoteCount");
            let key = format!("{}: {path}", self.name);
            if !told && n > 1 {
                if ALLOWED.iter().any(|(k, _)| *k == key) {
                    self.report.allowed.push(key);
                } else {
                    self.error(&format!(
                        "{path}: a list of {n} with neither totalCount nor pageInfo"
                    ));
                }
            }
            if last && descending {
                self.error(&format!(
                    "{path}: last with a descending order is the oldest, not the newest"
                ));
            }
        }
        self.selections(def.ty, field.selection_set(), &path, here);
    }

    fn error(&mut self, message: &str) {
        self.report.errors.push(format!("{}: {message}", self.name));
    }
}

/// Checks one query, returning its errors and worst-case node count.
fn check(schema: &Schema<'_>, name: &str, query: &str) -> Report {
    let doc = match cynic_parser::parse_executable_document(query) {
        Ok(doc) => doc,
        Err(err) => {
            return Report {
                errors: vec![format!("{name}: doesn't parse: {err}")],
                ..Report::default()
            };
        }
    };
    let mut walk = Walk {
        schema,
        doc: &doc,
        name,
        report: Report::default(),
    };
    for op in doc.operations() {
        let declared: HashSet<&str> = op.variable_definitions().map(|v| v.name()).collect();
        let root = match op.operation_type() {
            cynic_parser::common::OperationType::Mutation => "Mutation",
            _ => "Query",
        };
        walk.used_variables(op.selection_set(), &declared);
        walk.selections(root, op.selection_set(), "", 1);
    }
    walk.report
}

impl<'d> Walk<'_, 'd> {
    fn used_variables(&mut self, set: Iter<'d, Selection<'d>>, declared: &HashSet<&str>) {
        for selection in set {
            let (args, inner): (Vec<_>, _) = match selection {
                Selection::Field(f) => (
                    f.arguments().map(|a| a.value()).collect(),
                    f.selection_set(),
                ),
                Selection::InlineFragment(f) => (Vec::new(), f.selection_set()),
                Selection::FragmentSpread(_) => continue,
            };
            for value in args {
                for var in value.variables_used() {
                    if !declared.contains(var) {
                        self.error(&format!("${var} isn't declared"));
                    }
                }
            }
            self.used_variables(inner, declared);
        }
    }
}

macro_rules! typed {
    ($($kind:ident $query:ty, $vars:ty;)*) => {
        vec![$((
            stringify!($query),
            cynic::queries::build_executable_document::<$query, $vars>(
                cynic::queries::OperationType::$kind,
                None,
                HashSet::new(),
                None,
            ),
        )),*]
    };
}

/// The typed queries, as sent.
fn typed() -> Vec<(&'static str, String)> {
    use crate::browse as b;
    use crate::change as c;
    use crate::queries as q;
    typed! {
        Query q::RollupsQuery, q::NodesVariables;
        Query q::PullRequestQuery, q::NumberVariables;
        Query q::BehindQuery, q::BehindVariables;
        Query q::PrFilesQuery, q::PageVariables;
        Mutation q::MarkFileAsViewed, q::ViewedVariables;
        Mutation q::UnmarkFileAsViewed, q::ViewedVariables;
        Query q::ThreadsQuery, q::PageVariables;
        Query q::PendingReviewQuery, q::NumberVariables;
        Mutation q::StartReview, q::StartReviewVariables;
        Mutation q::AddThread, q::AddThreadVariables;
        Mutation q::SubmitReview, q::SubmitReviewVariables;
        Mutation q::Reply, q::ReplyVariables;
        Mutation q::Resolve, q::ThreadIdVariables;
        Mutation q::Unresolve, q::ThreadIdVariables;
        Query q::LastReviewQuery, q::LastReviewVariables;
        Query b::RepoQuery, b::RepoVariables;
        Query b::ObjectQuery, b::RepoVariables;
        Query b::BrowseSearch, b::BrowseSearchVariables;
        Query b::IssueQuery, q::NumberVariables;
        Query b::PrActivityQuery, q::NumberVariables;
        Query b::CommitQuery, b::RepoVariables;
        Query b::HistoryQuery, b::HistoryVariables;
        Query b::PrChecksQuery, q::NumberVariables;
        Query b::BranchChecksQuery, b::BranchesVariables;
        Query b::ContextsQuery, b::ContextsVariables;
        Query b::CommitChecksQuery, b::RepoVariables;
        Query b::ProfileQuery, b::ProfileVariables;
        Query b::ForksQuery, b::ListVariables;
        Query b::OwnerReposQuery, b::OwnerReposVariables;
        Query b::StarredQuery, b::LoginPageVariables;
        Query b::ReleasesQuery, b::ListVariables;
        Query b::ReleaseQuery, b::ReleaseVariables;
        Query b::TagsQuery, b::ListVariables;
        Mutation c::AddComment, c::AddCommentVariables;
        Mutation c::AddStar, c::IdVariables;
        Mutation c::RemoveStar, c::IdVariables;
        Mutation c::MergePr, c::MergeVariables;
        Mutation c::ClosePr, c::IdVariables;
        Mutation c::ReopenPr, c::IdVariables;
        Mutation c::CloseIssue, c::CloseIssueVariables;
        Mutation c::ReopenIssue, c::IdVariables;
        Mutation c::MarkReady, c::IdVariables;
        Mutation c::UpdateBranch, c::UpdateBranchVariables;
    }
}

#[test]
fn every_query_is_valid_and_says_what_it_leaves_out() {
    let text = include_str!("../../schema/github.graphql");
    let doc = cynic_parser::parse_type_system_document(text).unwrap();
    let schema = Schema::new(&doc, text);
    let mut errors = Vec::new();
    let mut allowed = HashSet::new();
    for (name, query) in typed().into_iter().chain(crate::raw::all()) {
        let report = check(&schema, name, &query);
        errors.extend(report.errors);
        allowed.extend(report.allowed);
        if report.nodes >= NODE_LIMIT {
            errors.push(format!("{name}: up to {} nodes", report.nodes));
        }
    }
    for (key, _) in ALLOWED {
        if !allowed.contains(*key) {
            errors.push(format!("{key}: allowed, but no longer needed"));
        }
    }
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// The checks above catch what they're meant to.
#[test]
fn the_check_catches_cut_lists_and_bad_fields() {
    let text = include_str!("../../schema/github.graphql");
    let doc = cynic_parser::parse_type_system_document(text).unwrap();
    let schema = Schema::new(&doc, text);
    let errors = |q: &str| check(&schema, "t", q).errors;
    assert!(errors("query { viewer { login } }").is_empty());
    let cut = errors("query { viewer { repositories(first: 10) { nodes { name } } } }");
    assert!(
        cut[0].contains("neither totalCount nor pageInfo"),
        "{cut:?}"
    );
    assert!(
        errors("query { viewer { repositories(first: 10) { totalCount nodes { name } } } }")
            .is_empty()
    );
    let backwards = errors(
        "query { viewer { repositories(last: 10, orderBy: {field: NAME, direction: DESC}) { totalCount } } }",
    );
    assert!(backwards[0].contains("oldest"), "{backwards:?}");
    // A contribution list is newest first unless asked otherwise.
    let by_default = errors(
        "query { viewer { contributionsCollection { issueContributions(last: 5) { totalCount } } } }",
    );
    assert!(by_default[0].contains("oldest"), "{by_default:?}");
    assert!(errors("query { viewer { nope } }")[0].contains("no field nope on User"));
    assert!(errors("query { viewer { login(x: 1) } }")[0].contains("takes no x"));
    assert!(
        errors("query { viewer { repositories(first: $n) { totalCount } } }")[0]
            .contains("$n isn't declared")
    );
    assert!(
        errors("query { viewer { repositories(first: 101) { totalCount } } }")[0]
            .contains("more than 100")
    );
    let huge = check(
        &schema,
        "t",
        "query { viewer { repositories(first: 100) { totalCount nodes { issues(first: 100) { totalCount nodes { comments(first: 100) { totalCount } } } } } } }",
    );
    assert_eq!(huge.nodes, 100 + 100 * 100 + 100 * 100 * 100);
}

/// Every typed query is in [`typed`], and every raw one in
/// [`crate::raw::all`].
#[test]
fn every_query_is_checked() {
    let source = [
        include_str!("queries.rs"),
        include_str!("browse.rs"),
        include_str!("change.rs"),
    ]
    .concat();
    let operations = source.matches("graphql_type = \"Query\"").count()
        + source.matches("graphql_type = \"Mutation\"").count();
    assert_eq!(typed().len(), operations);
    let raw = include_str!("raw.rs");
    let defined =
        raw.matches("\npub(crate) const ").count() + raw.matches("\npub(crate) fn ").count();
    let names: HashSet<&str> = crate::raw::all()
        .into_iter()
        .map(|(n, _)| n.split('(').next().unwrap_or(n))
        .collect();
    // `all` itself is a `pub(crate) fn`.
    assert_eq!(names.len(), defined - 1);
}
