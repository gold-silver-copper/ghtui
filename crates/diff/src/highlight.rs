//! Syntax highlighting with tree-sitter, producing per-line token spans.
//!
//! Grammars are behind `lang-*` cargo features. Unknown languages, files over
//! [`MAX_HIGHLIGHT_BYTES`] and grammar errors all fall back to plain text.

use std::sync::OnceLock;

use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::text::Text;

/// Larger files render without highlighting so they never stall a diff.
pub const MAX_HIGHLIGHT_BYTES: usize = 1 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenKind {
    Keyword,
    String,
    Comment,
    Function,
    Type,
    Number,
    Constant,
    Operator,
    Punctuation,
    Attribute,
    Property,
}

/// A highlighted byte range within one line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub kind: TokenKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Json,
    Yaml,
    Toml,
    Markdown,
    Bash,
}

impl Language {
    pub fn from_path(path: &str) -> Option<Language> {
        let name = path.rsplit('/').next().unwrap_or(path);
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        let lang = match (name, ext.as_deref()) {
            (_, Some("rs")) => Language::Rust,
            (_, Some("ts" | "mts" | "cts")) => Language::TypeScript,
            (_, Some("tsx")) => Language::Tsx,
            (_, Some("js" | "mjs" | "cjs" | "jsx")) => Language::JavaScript,
            (_, Some("py" | "pyi")) => Language::Python,
            (_, Some("go")) => Language::Go,
            (_, Some("json")) => Language::Json,
            (_, Some("yml" | "yaml")) => Language::Yaml,
            ("Cargo.lock" | "uv.lock" | "poetry.lock", _) | (_, Some("toml")) => Language::Toml,
            (_, Some("md" | "markdown")) => Language::Markdown,
            (".bashrc" | ".bash_profile" | ".zshrc" | ".profile", _)
            | (_, Some("sh" | "bash" | "zsh")) => Language::Bash,
            _ => return None,
        };
        Some(lang)
    }
}

/// Capture names we map, most specific first within a family. tree-sitter
/// picks the longest dotted prefix match.
const NAMES: &[(&str, TokenKind)] = &[
    ("keyword", TokenKind::Keyword),
    ("label", TokenKind::Keyword),
    ("tag", TokenKind::Keyword),
    ("conditional", TokenKind::Keyword),
    ("repeat", TokenKind::Keyword),
    ("include", TokenKind::Keyword),
    ("text.title", TokenKind::Keyword),
    ("markup.heading", TokenKind::Keyword),
    ("string", TokenKind::String),
    ("escape", TokenKind::String),
    ("text.literal", TokenKind::String),
    ("text.uri", TokenKind::String),
    ("comment", TokenKind::Comment),
    ("function", TokenKind::Function),
    ("constructor", TokenKind::Function),
    ("method", TokenKind::Function),
    ("type", TokenKind::Type),
    ("module", TokenKind::Type),
    ("namespace", TokenKind::Type),
    ("number", TokenKind::Number),
    ("float", TokenKind::Number),
    ("constant", TokenKind::Constant),
    ("boolean", TokenKind::Constant),
    ("variable.builtin", TokenKind::Constant),
    ("operator", TokenKind::Operator),
    ("punctuation", TokenKind::Punctuation),
    ("attribute", TokenKind::Attribute),
    ("property", TokenKind::Property),
];

fn config(lang: Language) -> Option<&'static HighlightConfiguration> {
    macro_rules! cached {
        ($feature:literal, $build:expr) => {{
            #[cfg(feature = $feature)]
            {
                static CONFIG: OnceLock<Option<HighlightConfiguration>> = OnceLock::new();
                CONFIG.get_or_init(|| build($build)).as_ref()
            }
            #[cfg(not(feature = $feature))]
            {
                None
            }
        }};
    }
    match lang {
        Language::Rust => cached!(
            "lang-rust",
            (
                tree_sitter_rust::LANGUAGE.into(),
                "rust",
                tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
            )
        ),
        Language::JavaScript => cached!(
            "lang-javascript",
            (
                tree_sitter_javascript::LANGUAGE.into(),
                "javascript",
                format!(
                    "{}\n{}",
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
                ),
            )
        ),
        // TypeScript's query extends JavaScript's.
        Language::TypeScript => cached!(
            "lang-typescript",
            (
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
                "typescript",
                format!(
                    "{}\n{}",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY
                ),
            )
        ),
        Language::Tsx => cached!(
            "lang-typescript",
            (
                tree_sitter_typescript::LANGUAGE_TSX.into(),
                "tsx",
                format!(
                    "{}\n{}\n{}",
                    tree_sitter_typescript::HIGHLIGHTS_QUERY,
                    tree_sitter_javascript::HIGHLIGHT_QUERY,
                    tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
                ),
            )
        ),
        Language::Python => cached!(
            "lang-python",
            (
                tree_sitter_python::LANGUAGE.into(),
                "python",
                tree_sitter_python::HIGHLIGHTS_QUERY.to_owned(),
            )
        ),
        Language::Go => cached!(
            "lang-go",
            (
                tree_sitter_go::LANGUAGE.into(),
                "go",
                tree_sitter_go::HIGHLIGHTS_QUERY.to_owned(),
            )
        ),
        Language::Json => cached!(
            "lang-json",
            (
                tree_sitter_json::LANGUAGE.into(),
                "json",
                tree_sitter_json::HIGHLIGHTS_QUERY.to_owned(),
            )
        ),
        Language::Yaml => cached!(
            "lang-yaml",
            (
                tree_sitter_yaml::LANGUAGE.into(),
                "yaml",
                tree_sitter_yaml::HIGHLIGHTS_QUERY.to_owned(),
            )
        ),
        Language::Toml => cached!(
            "lang-toml",
            (
                tree_sitter_toml_ng::LANGUAGE.into(),
                "toml",
                tree_sitter_toml_ng::HIGHLIGHTS_QUERY.to_owned(),
            )
        ),
        Language::Markdown => cached!(
            "lang-markdown",
            (
                tree_sitter_md::LANGUAGE.into(),
                "markdown",
                tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.to_owned(),
            )
        ),
        Language::Bash => cached!(
            "lang-bash",
            (
                tree_sitter_bash::LANGUAGE.into(),
                "bash",
                tree_sitter_bash::HIGHLIGHT_QUERY.to_owned(),
            )
        ),
    }
}

#[allow(dead_code)]
fn build(
    (language, name, query): (tree_sitter::Language, &str, String),
) -> Option<HighlightConfiguration> {
    match HighlightConfiguration::new(language, name, &query, "", "") {
        Ok(mut config) => {
            let names: Vec<&str> = NAMES.iter().map(|(n, _)| *n).collect();
            config.configure(&names);
            Some(config)
        }
        Err(err) => {
            // A grammar/query mismatch is a build problem, not a user one;
            // fall back to plain text.
            tracing::warn!(%err, "{name} highlight query failed");
            None
        }
    }
}

/// Token spans for each line of `text`. Returns an empty vector (plain text)
/// when the language isn't available or the file is too large.
pub fn highlight(lang: Option<Language>, text: &Text) -> Vec<Vec<Span>> {
    let Some(config) = lang.and_then(config) else {
        return Vec::new();
    };
    if text.source.len() > MAX_HIGHLIGHT_BYTES {
        return Vec::new();
    }
    let mut highlighter = Highlighter::new();
    let source = text.source.as_bytes();
    let Ok(events) = highlighter.highlight(config, source, None, None, |_| None) else {
        return Vec::new();
    };

    let mut out: Vec<Vec<Span>> = vec![Vec::new(); text.len()];
    let mut stack: Vec<TokenKind> = Vec::new();
    let mut line = 0usize;
    for event in events {
        let Ok(event) = event else {
            return Vec::new();
        };
        match event {
            HighlightEvent::HighlightStart(h) => stack.push(NAMES[h.0].1),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let Some(&kind) = stack.last() else { continue };
                let (start, end) = (start as u32, end as u32);
                // Advance to the line containing `start`, then split the
                // range across the lines it covers.
                while line < text.len()
                    && text.lines[line].1 < start
                    && line_end(text, line) <= start
                {
                    line += 1;
                }
                let mut l = line;
                while l < text.len() {
                    let (ls, le) = text.lines[l];
                    if ls >= end {
                        break;
                    }
                    let s = start.max(ls);
                    let e = end.min(le);
                    if s < e {
                        out[l].push(Span {
                            start: s - ls,
                            end: e - ls,
                            kind,
                        });
                    }
                    l += 1;
                }
            }
        }
    }
    out
}

/// Offset just past line `i`'s terminator.
fn line_end(text: &Text, i: usize) -> u32 {
    text.lines
        .get(i + 1)
        .map_or(text.source.len() as u32, |next| next.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(lang: Language, src: &str, line: usize) -> Vec<(String, TokenKind)> {
        let text = Text::new(src.as_bytes());
        let spans = highlight(Some(lang), &text);
        spans[line]
            .iter()
            .map(|s| {
                (
                    text.line(line)[s.start as usize..s.end as usize].to_owned(),
                    s.kind,
                )
            })
            .collect()
    }

    #[test]
    fn detects_languages() {
        assert_eq!(Language::from_path("src/main.rs"), Some(Language::Rust));
        assert_eq!(Language::from_path("a/b.TSX"), Some(Language::Tsx));
        assert_eq!(Language::from_path("Cargo.lock"), Some(Language::Toml));
        assert_eq!(Language::from_path("x/.bashrc"), Some(Language::Bash));
        assert_eq!(Language::from_path("README"), None);
        assert_eq!(Language::from_path("image.png"), None);
    }

    #[test]
    fn highlights_rust() {
        let spans = kinds(
            Language::Rust,
            "fn main() {\n    let s = \"hi\"; // note\n}\n",
            1,
        );
        assert!(
            spans.contains(&("let".into(), TokenKind::Keyword)),
            "{spans:?}"
        );
        assert!(
            spans.contains(&("\"hi\"".into(), TokenKind::String)),
            "{spans:?}"
        );
        assert!(
            spans.contains(&("// note".into(), TokenKind::Comment)),
            "{spans:?}"
        );
    }

    #[test]
    fn multi_line_tokens_split_per_line() {
        let src = "/* one\ntwo */\nfn f() {}\n";
        assert_eq!(
            kinds(Language::Rust, src, 0),
            [("/* one".into(), TokenKind::Comment)]
        );
        assert_eq!(
            kinds(Language::Rust, src, 1),
            [("two */".into(), TokenKind::Comment)]
        );
    }

    #[test]
    fn every_bundled_grammar_loads() {
        for lang in [
            Language::Rust,
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Python,
            Language::Go,
            Language::Json,
            Language::Yaml,
            Language::Toml,
            Language::Markdown,
            Language::Bash,
        ] {
            assert!(config(lang).is_some(), "{lang:?}");
        }
    }

    #[test]
    fn typescript_and_python() {
        let ts = kinds(Language::TypeScript, "const x: number = 1;\n", 0);
        assert!(ts.contains(&("const".into(), TokenKind::Keyword)), "{ts:?}");
        let py = kinds(Language::Python, "def f():\n    return 'a'\n", 1);
        assert!(
            py.contains(&("return".into(), TokenKind::Keyword)),
            "{py:?}"
        );
        assert!(py.contains(&("'a'".into(), TokenKind::String)), "{py:?}");
    }

    #[test]
    fn crlf_lines_and_plain_fallback() {
        let spans = kinds(Language::Rust, "let a = 1;\r\nlet b = 2;\r\n", 1);
        assert_eq!(spans.first().map(|s| s.0.as_str()), Some("let"));
        assert!(highlight(None, &Text::new(b"x\n")).is_empty());
    }
}
