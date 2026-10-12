//! Just enough YAML for issue templates: their front matter, issue forms
//! and `config.yml`. Block mappings and sequences by indentation, flow
//! lists (`[a, b]`) and maps, quoted and plain scalars, `|` and `>` block
//! scalars, and comments. Anchors, tags and multi-line plain scalars
//! aren't templates' YAML; what isn't understood reads as text or nothing.

/// A YAML value. Scalars stay text: a template's `true` is read as it's
/// used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Null,
    Text(String),
    List(Vec<Value>),
    Map(Vec<(String, Value)>),
}

impl Value {
    /// The value at `key` of a map.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn list(&self) -> &[Value] {
        match self {
            Value::List(items) => items,
            _ => &[],
        }
    }

    /// Names given either way GitHub takes them: a list, or one string of
    /// names between commas.
    pub fn names(&self) -> Vec<String> {
        let names: Vec<String> = match self {
            Value::Text(text) => text.split(',').map(str::to_owned).collect(),
            Value::List(items) => items
                .iter()
                .filter_map(Value::text)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        (names.into_iter())
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .collect()
    }

    /// `false` (or `no`) said in so many words.
    pub fn is_false(&self) -> bool {
        matches!(self.text(), Some("false" | "False" | "FALSE" | "no" | "No"))
    }
}

/// Parses a YAML document.
pub fn parse(text: &str) -> Value {
    let mut lines: Vec<String> = text
        .lines()
        .map(|l| l.trim_end_matches('\r').to_owned())
        .collect();
    // A document start marker opens it; an end marker closes it.
    if lines.first().is_some_and(|l| l.trim_end() == "---") {
        lines.remove(0);
    }
    if let Some(end) = lines.iter().position(|l| l.trim_end() == "...") {
        lines.truncate(end);
    }
    let mut parser = Parser { lines, at: 0 };
    parser.block(0)
}

struct Parser {
    lines: Vec<String>,
    at: usize,
}

/// Columns of indentation.
fn indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// Whether a line holds nothing: blank, or only a comment.
fn empty(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

/// The line without its comment (a `#` after a space, outside quotes).
fn uncommented(line: &str) -> &str {
    let mut quote = None;
    let mut previous = ' ';
    for (i, c) in line.char_indices() {
        match (quote, c) {
            (None, '"' | '\'') if previous == ' ' || previous == '[' || previous == ',' => {
                quote = Some(c);
            }
            (Some(q), c) if c == q => quote = None,
            (None, '#') if previous == ' ' || previous == '\t' => {
                return line.get(..i).unwrap_or(line).trim_end();
            }
            _ => {}
        }
        previous = c;
    }
    line.trim_end()
}

/// A sequence item's dash: `-` alone or before a space.
fn dash(content: &str) -> Option<&str> {
    match content.strip_prefix('-')? {
        "" => Some(""),
        rest if rest.starts_with(' ') => Some(rest),
        _ => None,
    }
}

/// `key: value` split at its colon (outside quotes): the key, unquoted,
/// and what follows.
fn key_value(content: &str) -> Option<(String, &str)> {
    let first = content.chars().next()?;
    if first == '"' || first == '\'' {
        let close = content.get(1..)?.find(first)? + 1;
        let key = content.get(1..close)?;
        let rest = content.get(close + 1..)?.trim_start();
        let rest = rest.strip_prefix(':')?;
        return (rest.is_empty() || rest.starts_with(' ')).then(|| (key.to_owned(), rest.trim()));
    }
    let mut from = 0;
    loop {
        let colon = from + content.get(from..)?.find(':')?;
        let rest = content.get(colon + 1..)?;
        if rest.is_empty() || rest.starts_with(' ') {
            let key = content.get(..colon)?.trim();
            return (!key.is_empty()).then(|| (key.to_owned(), rest.trim()));
        }
        from = colon + 1;
    }
}

impl Parser {
    /// The next line with something on it, and its indentation.
    fn peek(&mut self) -> Option<(usize, String)> {
        while let Some(line) = self.lines.get(self.at) {
            if empty(line) {
                self.at += 1;
                continue;
            }
            return Some((indent(line), uncommented(line).trim_start().to_owned()));
        }
        None
    }

    /// The node starting at the next line, if it's at least `min` in.
    fn block(&mut self, min: usize) -> Value {
        let Some((at, content)) = self.peek() else {
            return Value::Null;
        };
        if at < min {
            return Value::Null;
        }
        if dash(&content).is_some() {
            self.list(at)
        } else if key_value(&content).is_some() {
            self.map(at)
        } else {
            self.at += 1;
            scalar(&content)
        }
    }

    fn list(&mut self, at: usize) -> Value {
        let mut items = Vec::new();
        while let Some((here, content)) = self.peek() {
            let Some(rest) = dash(&content).filter(|_| here == at) else {
                break;
            };
            let item = rest.trim_start();
            if item.is_empty() {
                self.at += 1;
                items.push(self.block(at + 1));
            } else if key_value(item).is_some() {
                // `- key: value` starts a map where `key` is: that line,
                // re-read without its dash.
                let column = at + content.len() - item.len();
                if let Some(line) = self.lines.get_mut(self.at) {
                    *line = format!("{}{item}", " ".repeat(column));
                }
                items.push(self.map(column));
            } else {
                self.at += 1;
                items.push(self.inline(item, at));
            }
        }
        Value::List(items)
    }

    fn map(&mut self, at: usize) -> Value {
        let mut entries = Vec::new();
        while let Some((here, content)) = self.peek() {
            if here != at || dash(&content).is_some() {
                break;
            }
            let Some((key, value)) = key_value(&content) else {
                break;
            };
            self.at += 1;
            let value = if value.is_empty() {
                // A list may sit at the key's own indentation.
                match self.peek() {
                    Some((next, c)) if next == at && dash(&c).is_some() => self.list(at),
                    _ => self.block(at + 1),
                }
            } else {
                self.inline(value, at)
            };
            entries.push((key, value));
        }
        Value::Map(entries)
    }

    /// A value on its key's or dash's line: a block scalar's header, a
    /// flow list or map, or a scalar.
    fn inline(&mut self, value: &str, at: usize) -> Value {
        if let Some(style) = value.strip_prefix(['|', '>']) {
            let folded = value.starts_with('>');
            return Value::Text(self.block_scalar(at, folded, style));
        }
        flow(value)
    }

    /// The lines of a `|` or `>` scalar: those indented past `at`.
    fn block_scalar(&mut self, at: usize, folded: bool, style: &str) -> String {
        let mut raw = Vec::new();
        while let Some(line) = self.lines.get(self.at) {
            if !line.trim().is_empty() && indent(line) <= at {
                break;
            }
            raw.push(line.as_str());
            self.at += 1;
        }
        // Indented as its first line with something on it.
        let column = (raw.iter())
            .find(|l| !l.trim().is_empty())
            .map_or(0, |l| indent(l));
        let mut lines: Vec<String> = (raw.iter())
            .map(|l| l.get(column.min(l.len())..).unwrap_or("").to_owned())
            .collect();
        // Blank lines at the end belong to what follows, unless kept.
        let mut trailing = 0;
        while lines.last().is_some_and(|l| l.trim().is_empty()) {
            lines.pop();
            trailing += 1;
        }
        let mut text = if folded {
            let mut out = String::new();
            for line in &lines {
                if line.is_empty() {
                    out.push('\n');
                } else {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push(' ');
                    }
                    out.push_str(line);
                }
            }
            out
        } else {
            lines.join("\n")
        };
        if style.contains('-') || text.is_empty() {
            return text;
        }
        text.push('\n');
        if style.contains('+') {
            text.push_str(&"\n".repeat(trailing));
        }
        text
    }
}

/// A flow list (`[a, "b"]`), a flow map (`{a: 1}`), or a scalar.
fn flow(value: &str) -> Value {
    let inner = |open: char, close: char| {
        let inner = value.strip_prefix(open)?.strip_suffix(close)?;
        Some(split_commas(inner))
    };
    if let Some(items) = inner('[', ']') {
        return Value::List(items.iter().map(|i| scalar(i)).collect());
    }
    if let Some(items) = inner('{', '}') {
        let entries = items.iter().filter_map(|i| {
            let (key, value) = key_value(i)?;
            Some((key, scalar(value)))
        });
        return Value::Map(entries.collect());
    }
    scalar(value)
}

/// Items between commas, outside quotes; empty ones left out.
fn split_commas(inner: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut item = String::new();
    let mut quote = None;
    for c in inner.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, ',') => {
                items.push(std::mem::take(&mut item));
                continue;
            }
            _ => {}
        }
        item.push(c);
    }
    items.push(item);
    (items.into_iter())
        .map(|i| i.trim().to_owned())
        .filter(|i| !i.is_empty())
        .collect()
}

/// A scalar: quoted (with its escapes), `~`/`null`, or plain text.
fn scalar(value: &str) -> Value {
    let value = value.trim();
    if let Some(inner) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => {}
            }
        }
        return Value::Text(out);
    }
    if let Some(inner) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        return Value::Text(inner.replace("''", "'"));
    }
    match value {
        "" | "~" | "null" | "Null" | "NULL" => Value::Null,
        _ => Value::Text(value.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Value {
        Value::Text(s.to_owned())
    }

    /// An issue form as GitHub documents them: nested maps in a list,
    /// flow lists, quoted strings, block scalars and comments.
    #[test]
    fn reads_an_issue_form() {
        let form = parse(
            r#"name: Bug Report   # shown in the chooser
description: "File a bug: it's broken"
title: '[Bug]: '
labels: ["bug", triage]
assignees:
  - octocat
body:
  - type: markdown
    attributes:
      value: |
        Thanks for taking the time!

        Really.
  - type: textarea
    id: what
    attributes:
      label: What happened?
      placeholder: Tell us # not a comment inside? it is
      render: shell
    validations:
      required: true
  - type: dropdown
    attributes:
      label: Version
      options:
      - "1.0"
      - 2.0
"#,
        );
        assert_eq!(form.get("name"), Some(&text("Bug Report")));
        assert_eq!(
            form.get("description"),
            Some(&text("File a bug: it's broken"))
        );
        assert_eq!(form.get("title"), Some(&text("[Bug]: ")));
        assert_eq!(
            form.get("labels").map(Value::names),
            Some(vec!["bug".into(), "triage".into()])
        );
        assert_eq!(
            form.get("assignees").map(Value::names),
            Some(vec!["octocat".into()])
        );
        let body = form.get("body").map(Value::list).unwrap_or_default();
        assert_eq!(body.len(), 3);
        let value = body[0].get("attributes").and_then(|a| a.get("value"));
        assert_eq!(
            value,
            Some(&text("Thanks for taking the time!\n\nReally.\n"))
        );
        let attributes = body[1].get("attributes").unwrap();
        assert_eq!(attributes.get("label"), Some(&text("What happened?")));
        assert_eq!(attributes.get("render"), Some(&text("shell")));
        let required = body[1].get("validations").and_then(|v| v.get("required"));
        assert_eq!(required, Some(&text("true")));
        let options = body[2].get("attributes").and_then(|a| a.get("options"));
        assert_eq!(
            options.map(Value::names),
            Some(vec!["1.0".into(), "2.0".into()])
        );
    }

    #[test]
    fn reads_front_matter_and_config() {
        let front = parse(
            "name: Feature\nabout: Suggest an idea\nlabels: enhancement, help wanted\nassignees: ''\n",
        );
        assert_eq!(
            front.get("labels").map(Value::names),
            Some(vec!["enhancement".into(), "help wanted".into()])
        );
        assert_eq!(front.get("assignees").map(Value::names), Some(Vec::new()));
        let config = parse(
            "blank_issues_enabled: false\ncontact_links:\n  - name: Forum\n    url: https://example.com\n    about: Ask here\n",
        );
        assert!(
            config
                .get("blank_issues_enabled")
                .is_some_and(Value::is_false)
        );
        let links = config
            .get("contact_links")
            .map(Value::list)
            .unwrap_or_default();
        assert_eq!(links[0].get("url"), Some(&text("https://example.com")));
        assert_eq!(links[0].get("about"), Some(&text("Ask here")));
    }

    #[test]
    fn folds_and_chomps_block_scalars() {
        let v = parse("a: >-\n  one\n  two\n\n  three\nb: |+\n  kept\n\nc: x\n");
        assert_eq!(v.get("a"), Some(&text("one two\nthree")));
        assert_eq!(v.get("b"), Some(&text("kept\n\n")));
        assert_eq!(v.get("c"), Some(&text("x")));
    }
}
