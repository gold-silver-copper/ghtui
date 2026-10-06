//! Named scopes (functions, methods, types, impls, modules) and the lines
//! they span, from the syntax tree, so a hunk can say where it is:
//! `impl Doc › fn offset`.

use std::ops::{ControlFlow, RangeInclusive};
use std::time::{Duration, Instant};

use tree_sitter::{Node, ParseOptions, Parser};

use crate::highlight::{Language, MAX_HIGHLIGHT_BYTES, config};
use crate::sat_u32;
use crate::text::Text;

/// How long parsing for scopes may take.
const PARSE_BUDGET: Duration = Duration::from_secs(1);

/// A named scope and the lines it spans (1-based, inclusive).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub lines: RangeInclusive<u32>,
    pub name: String,
}

/// Every named scope in `text`, outermost first (so the scopes containing
/// a line, in order, are its path). Empty for languages without scope
/// names, files too big to highlight, and anything that won't parse in
/// time. A syntax error only loses the scopes it breaks.
pub fn scopes(lang: Option<Language>, text: &Text) -> Vec<Scope> {
    let Some((lang, config)) = lang.and_then(|l| Some((l, config(l)?))) else {
        return Vec::new();
    };
    if text.source.len() > MAX_HIGHLIGHT_BYTES {
        return Vec::new();
    }
    let mut parser = Parser::new();
    if parser.set_language(&config.language).is_err() {
        return Vec::new();
    }
    let source = text.source.as_bytes();
    let start = Instant::now();
    let mut in_time = |_: &tree_sitter::ParseState| {
        if start.elapsed() > PARSE_BUDGET {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = ParseOptions::new().progress_callback(&mut in_time);
    let Some(tree) = parser.parse_with_options(
        &mut |at, _| source.get(at..).unwrap_or_default(),
        None,
        Some(options),
    ) else {
        return Vec::new();
    };
    // Pre-order walk: parents before children.
    let mut out = Vec::new();
    let mut cursor = tree.walk();
    'walk: loop {
        let node = cursor.node();
        if let Some(name) = name(lang, node, source) {
            let row = |p: tree_sitter::Point| sat_u32(p.row).saturating_add(1);
            out.push(Scope {
                lines: row(node.start_position())..=row(node.end_position()),
                name,
            });
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    out
}

/// How `node` is named in a scope path, if it's a scope at all.
fn name(lang: Language, node: Node<'_>, source: &[u8]) -> Option<String> {
    use Language::{Go, JavaScript, Python, Rust, Tsx, TypeScript};
    let field = |f: &str| {
        let text = node.child_by_field_name(f)?.utf8_text(source).ok()?;
        // Generic parameters and the like can span lines.
        Some(text.split_whitespace().collect::<Vec<_>>().join(" "))
    };
    let js = matches!(lang, JavaScript | TypeScript | Tsx);
    let keyword = match (lang, node.kind()) {
        (Rust, "impl_item") => {
            return Some(match field("trait") {
                Some(t) => format!("impl {t} for {}", field("type")?),
                None => format!("impl {}", field("type")?),
            });
        }
        // Methods, and `const handler = () => {...}`: just the name.
        (_, "method_definition") if js => return field("name"),
        (_, "variable_declarator")
            if js
                && node.child_by_field_name("value").is_some_and(|v| {
                    matches!(
                        v.kind(),
                        "arrow_function" | "function_expression" | "function"
                    )
                }) =>
        {
            return field("name");
        }
        (Rust, "function_item" | "function_signature_item") => "fn",
        (Rust, "struct_item") => "struct",
        (Rust, "enum_item") | (TypeScript | Tsx, "enum_declaration") => "enum",
        (Rust, "union_item") => "union",
        (Rust, "trait_item") => "trait",
        (Rust, "mod_item") => "mod",
        (Rust, "macro_definition") => "macro_rules!",
        (Python, "function_definition") => "def",
        (Python, "class_definition") => "class",
        (_, "class_declaration" | "abstract_class_declaration") if js => "class",
        (Go, "function_declaration" | "method_declaration") => "func",
        (Go, "type_spec") => "type",
        (_, "function_declaration" | "generator_function_declaration") if js => "function",
        (TypeScript | Tsx, "interface_declaration") => "interface",
        (TypeScript | Tsx, "internal_module") => "namespace",
        _ => return None,
    };
    Some(format!("{keyword} {}", field("name")?))
}

/// The path of scopes containing `line`, outermost first.
pub fn path(scopes: &[Scope], line: u32) -> Vec<&str> {
    scopes
        .iter()
        .filter(|s| s.lines.contains(&line))
        .map(|s| s.name.as_str())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(lang: Language, source: &str, line: u32) -> Vec<String> {
        let text = Text::new(source.as_bytes());
        path(&scopes(Some(lang), &text), line)
            .into_iter()
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn rust_methods_in_impls() {
        let src = "struct Doc;\n\nimpl Doc {\n    fn offset(&self) -> u32 {\n        1\n    }\n}\n\nimpl Display for Doc {\n    fn fmt(&self) {}\n}\n";
        assert_eq!(at(Language::Rust, src, 5), ["impl Doc", "fn offset"]);
        assert_eq!(
            at(Language::Rust, src, 10),
            ["impl Display for Doc", "fn fmt"]
        );
        assert_eq!(at(Language::Rust, src, 2), Vec::<String>::new());
    }

    #[test]
    fn nested_functions_nest() {
        let src = "mod a {\n    fn outer() {\n        fn inner() {\n            x();\n        }\n    }\n}\n";
        assert_eq!(
            at(Language::Rust, src, 4),
            ["mod a", "fn outer", "fn inner"]
        );
    }

    #[test]
    fn other_languages() {
        let py = "class Shape:\n    def area(self):\n        return 1\n";
        assert_eq!(at(Language::Python, py, 3), ["class Shape", "def area"]);
        let go = "package p\n\nfunc (s *Shape) Area() int {\n\treturn 1\n}\n";
        assert_eq!(at(Language::Go, go, 4), ["func Area"]);
        let ts = "class Shape {\n  area(): number {\n    return 1;\n  }\n}\nconst run = () => {\n  go();\n};\n";
        assert_eq!(at(Language::TypeScript, ts, 3), ["class Shape", "area"]);
        assert_eq!(at(Language::TypeScript, ts, 7), ["run"]);
    }

    /// A syntax error loses what it breaks, not everything.
    #[test]
    fn broken_code_still_names_what_parses() {
        let src = "fn fine() {\n    ok();\n}\n\nfn broken( {\n";
        assert_eq!(at(Language::Rust, src, 2), ["fn fine"]);
    }

    #[test]
    fn plain_text_has_no_scopes() {
        assert!(scopes(None, &Text::new(b"fn x() {}\n")).is_empty());
    }
}
