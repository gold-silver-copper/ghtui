//! Every GraphQL query ghtui sends, checked against GitHub's schema and
//! for lists cut short without saying so.
//!
//! The typed (cynic) queries are checked against the schema when they
//! compile, the raw ones ([`crate::api::raw`]) only here. Both are checked here
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
/// 1`, is a "the newest" or "the first", not a list, and needs none.) One
/// row each, so rustfmt is kept off.
#[rustfmt::skip]
const ALLOWED: &[(&str, &str)] = &[
    ("q::LastReviewQuery: repository.pullRequest.reviews", "Your newest submitted review: you have at most one pending review on a PR, so your last 20 include it"),
    ("b::ProfileQuery: user.pinnedItems", "GitHub pins at most 6"),
    ("b::ProfileQuery: organization.pinnedItems", "GitHub pins at most 6"),
    ("b::ProfileQuery: user.socialAccounts", "GitHub keeps at most 4 social accounts"),
    ("TOPIC: topic.relatedTopics", "A topic's related topics: GitHub relates at most 10"),
    ("BRANCHES: repository.refs.nodes.associatedPullRequests", "Only the newest from this branch in this repository is shown; 10 newer ones from forks' branches of the same name is not worth paging for"),
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
    use crate::api::browse as b;
    use crate::api::change as c;
    use crate::api::edit as e;
    use crate::api::queries as q;
    use crate::api::timeline as t;
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
        Query b::StateCountsQuery, b::StateCountsVariables;
        Query b::IssueQuery, q::NumberVariables;
        Query b::PrActivityQuery, q::NumberVariables;
        Query b::CommitQuery, b::RepoVariables;
        Query b::HistoryQuery, b::HistoryVariables;
        Query b::PrChecksQuery, q::NumberVariables;
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
        Mutation c::EnableAutoMerge, c::MergeVariables;
        Mutation c::DisableAutoMerge, c::IdVariables;
        Mutation c::Enqueue, c::EnqueueVariables;
        Mutation c::Dequeue, c::IdVariables;
        Mutation c::ClosePr, c::IdVariables;
        Mutation c::ReopenPr, c::IdVariables;
        Mutation c::CloseIssue, c::CloseIssueVariables;
        Mutation c::ReopenIssue, c::IdVariables;
        Mutation c::MarkReady, c::IdVariables;
        Mutation c::UpdateBranch, c::UpdateBranchVariables;
        Query e::SidebarQuery, q::NumberVariables;
        Query e::TeamsQuery, e::TeamsVariables;
        Mutation e::UpdateIssueBody, e::BodyVariables;
        Mutation e::UpdatePrBody, e::BodyVariables;
        Mutation e::UpdateIssueComment, e::BodyVariables;
        Mutation e::UpdateReviewComment, e::BodyVariables;
        Mutation e::DeleteIssueComment, c::IdVariables;
        Mutation e::DeleteReviewComment, c::IdVariables;
        Mutation e::UpdateReview, e::BodyVariables;
        Mutation e::DeleteReview, c::IdVariables;
        Query t::ReviewsQuery, q::PageVariables;
        Mutation e::UpdateIssueTitle, e::TitleVariables;
        Mutation e::UpdatePrTitle, e::TitleVariables;
        Mutation e::AddReaction, e::ReactVariables;
        Mutation e::RemoveReaction, e::ReactVariables;
        Mutation e::AddLabels, e::AddVariables;
        Mutation e::RemoveLabels, e::RemoveVariables;
        Mutation e::AddAssignees, e::AddVariables;
        Mutation e::RemoveAssignees, e::RemoveVariables;
        Mutation e::RequestReviews, e::ReviewersVariables;
        Mutation e::SetIssueMilestone, e::MilestoneVariables;
        Mutation e::SetPrMilestone, e::MilestoneVariables;
        Mutation crate::api::create::CreateIssue, crate::api::create::CreateIssueVariables;
        Mutation crate::api::create::CreatePr, crate::api::create::CreatePrVariables;
        Query crate::api::watch::RepoWatchQuery, crate::api::watch::RepoNameVariables;
        Query crate::api::watch::ThreadWatchQuery, q::NumberVariables;
        Query crate::api::watch::DiscussionWatchQuery, q::NumberVariables;
        Mutation crate::api::watch::UpdateSubscription, crate::api::watch::SubscribeVariables;
        Query crate::api::release::TagQuery, crate::api::release::TagVariables;
    }
}

#[test]
fn every_query_is_valid_and_says_what_it_leaves_out() {
    let text = include_str!(env!("GITHUB_SDL"));
    let doc = cynic_parser::parse_type_system_document(text).unwrap();
    let schema = Schema::new(&doc, text);
    let mut errors = Vec::new();
    let mut allowed = HashSet::new();
    for (name, query) in typed().into_iter().chain(crate::api::raw::all()) {
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

/// Every query, without what the oldest Enterprise Server lacks, is
/// still a valid query: what's left out leaves no empty selection or
/// variable behind, and what stands in is a field GitHub has.
#[test]
fn every_query_without_what_an_old_server_lacks_is_valid() {
    use crate::api::enterprise::{Server, Version, lacks, rewrite};
    let text = include_str!(env!("GITHUB_SDL"));
    let doc = cynic_parser::parse_type_system_document(text).unwrap();
    let schema = Schema::new(&doc, text);
    let old = lacks(Server {
        version: Some(Version(3, 9)),
    });
    let mut errors = Vec::new();
    let mut rewritten = 0;
    for (name, query) in typed().into_iter().chain(crate::api::raw::all()) {
        let Some(done) = rewrite(&query, &old) else { continue };
        rewritten += 1;
        errors.extend(check(&schema, name, &done.query).errors);
        let doc = cynic_parser::parse_executable_document(&done.query).unwrap();
        for op in doc.operations() {
            for var in op.variable_definitions() {
                let used = done.query.matches(&format!("${}", var.name())).count() > 1;
                if !used {
                    errors.push(format!("{name}: ${} is declared but not used", var.name()));
                }
            }
        }
    }
    assert!(rewritten > 5, "only {rewritten} queries lack anything");
    assert!(errors.is_empty(), "{}", errors.join("\n"));
}

/// What an Enterprise Server's schema lacks of what ghtui asks: run with
/// `GHTUI_GHES_SDL` naming one of the schemas GitHub publishes (in
/// github/docs, `src/graphql/data/ghes-3.N/schema.docs-enterprise.graphql`),
/// it lists them, to keep [`crate::api::enterprise`]'s table up to date.
#[test]
#[ignore = "reads an Enterprise Server's schema named by GHTUI_GHES_SDL"]
#[expect(clippy::print_stdout, reason = "its report is its output")]
fn what_an_enterprise_schema_lacks() {
    let path = std::env::var("GHTUI_GHES_SDL").unwrap();
    let text = std::fs::read_to_string(path).unwrap();
    let doc = cynic_parser::parse_type_system_document(&text).unwrap();
    let schema = Schema::new(&doc, &text);
    // With its version in GHTUI_GHES_VERSION, what ghtui leaves out for
    // it is left out first: then nothing should be listed.
    let version = std::env::var("GHTUI_GHES_VERSION").ok();
    let server = crate::api::enterprise::Server {
        version: version.and_then(|v| crate::api::enterprise::Version::parse(&v)),
    };
    let left_out = crate::api::enterprise::lacks(server);
    let mut lacks = std::collections::BTreeSet::new();
    for (name, query) in typed().into_iter().chain(crate::api::raw::all()) {
        let query = match server.version {
            Some(_) => {
                crate::api::enterprise::rewrite(&query, &left_out).map_or(query, |r| r.query)
            }
            None => query,
        };
        for error in check(&schema, name, &query).errors {
            // Only what's missing: the rest is checked against github.com.
            if let Some((_, what)) = error.split_once(": no ").or(error.split_once(": takes no")) {
                lacks.insert(format!("{what}    ({error})"));
            } else if error.contains("takes no") {
                lacks.insert(error);
            }
        }
    }
    for line in lacks {
        println!("{line}");
    }
}

/// The checks above catch what they're meant to.
#[test]
fn the_check_catches_cut_lists_and_bad_fields() {
    let text = include_str!(env!("GITHUB_SDL"));
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
/// [`crate::api::raw::all`].
#[test]
fn every_query_is_checked() {
    let source = [
        include_str!("queries.rs"),
        include_str!("browse.rs"),
        include_str!("change.rs"),
        include_str!("edit.rs"),
        include_str!("create.rs"),
        include_str!("watch.rs"),
        include_str!("release.rs"),
        include_str!("timeline.rs"),
    ]
    .concat();
    // The rows of `fragments! { root "Query" .. }` tables are queries too.
    let roots: usize = (source.split("root \"Query\"").skip(1))
        .map(|table| {
            table
                .split('}')
                .next()
                .unwrap_or("")
                .matches(") =>")
                .count()
        })
        .sum();
    // `mutations!` writes `graphql_type = "Mutation"` once for all its
    // rows, which it counts itself.
    let operations = source.matches("graphql_type = \"Query\"").count()
        + source.matches("graphql_type = \"Mutation\"").count()
        - 1
        + crate::api::change::MUTATIONS
        + crate::api::edit::MUTATIONS
        + crate::api::create::MUTATIONS
        + crate::api::watch::MUTATIONS
        + roots;
    assert_eq!(typed().len(), operations);
    // The text queries are listed by `raw!`, but those built at run time
    // by hand: each `pub(crate) fn` but `all` itself.
    let built = include_str!("raw.rs").matches("\npub(crate) fn ").count() - 1;
    let names: HashSet<&str> = crate::api::raw::all()
        .into_iter()
        .map(|(n, _)| n.split('(').next().unwrap_or(n))
        .collect();
    let consts = crate::api::raw::RAW.len()
        + crate::api::housekeeping::RAW.len()
        + crate::api::org::RAW.len();
    assert_eq!(names.len(), consts + built);
}
