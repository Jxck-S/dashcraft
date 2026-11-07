//! Completion for M text.

use std::collections::HashSet;

use crate::ast::PrimitiveType;
use crate::lexer::{Keyword, TokenKind, is_identifier_part, is_identifier_start};
use crate::span::Span;

use super::environment::{Environment, EnvironmentKind, function_shape};
use super::help::floor_char_boundary;
use super::library::{LibraryKind, catalog};
use super::names::{
    CONTEXTUAL_KEYWORDS, NameKind, constructor_shape, keyword_description, primitive_description,
    quality, signature,
};
use super::scan::{Local, Position, Scan, scan};

/// One suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    /// The name (`Changed Type`, `Table.AddColumn`, `#date`, `let`).
    pub label: String,
    pub kind: NameKind,
    /// The text that replaces [`Completions::span`]: the name as M writes it
    /// (`#"Changed Type"`).
    pub insert_text: String,
    /// A function's signature; otherwise what the name is (`step`, `parameter (number)`,
    /// `query parameter (Text)`, `type (number)`, `keyword`).
    pub detail: String,
    /// What a keyword or primitive type is, or the model's description of a query.
    pub description: Option<String>,
    /// Whether DashCraft evaluates a library name.
    pub supported: bool,
}

/// The suggestions at a cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Completions {
    /// The text a chosen item replaces: the name being typed (a whole `#"…"` in one), or an
    /// empty span at the cursor.
    pub span: Span,
    /// The typed text the items match (`Sou` in `Sou`, `Changed` in `#"Changed`).
    pub prefix: String,
    /// Best match first.
    pub items: Vec<CompletionItem>,
    /// More items matched than the limit allowed.
    pub incomplete: bool,
}

/// The completions at `offset` (a byte offset; clamped to the text), best first and at most
/// `max_items` (0: no limit).
///
/// In an expression: the steps, parameters and fields in scope (innermost first, as Power BI
/// Desktop binds them), then the queries of `env`, the keywords and the standard library. After
/// `as`, `is`, `type` or `nullable`: the primitive types; in the type of a field of a record or
/// table type: the primitive types, then the names of an expression. Matches ignore case and rank
/// an exact name first, then names that start with the typed text, then a word of the name that
/// does, then names that contain it. There are none in a comment, text or number, where a step,
/// member or field name is being written, and inside `[…]` field access.
#[must_use]
pub fn complete(src: &str, offset: usize, env: &Environment, max_items: usize) -> Completions {
    let offset = floor_char_boundary(src, offset);
    let full = scan(src, Some(offset));
    let empty = Completions {
        span: Span::new(offset, offset),
        ..Completions::default()
    };
    let Some(cx) = context(&full, src, offset) else {
        return empty;
    };
    let at = if cx.span.start == offset {
        full.at_cursor
    } else {
        scan(src, Some(cx.span.start)).at_cursor
    };
    let position = at.map_or(Position::Expression, |a| a.position);
    let mut c = Collector {
        prefix: cx.prefix.to_lowercase(),
        out: Vec::new(),
    };
    match position {
        Position::Declaration | Position::FieldName => {}
        Position::Type | Position::FieldType => {
            // Primitive types come before the names of an expression at the same match quality.
            let rank = u8::from(position == Position::Type);
            for p in PrimitiveType::ALL {
                let name = p.name();
                c.offer(rank, name, || CompletionItem {
                    label: name.to_owned(),
                    kind: NameKind::Type,
                    insert_text: name.to_owned(),
                    detail: "type".to_owned(),
                    description: primitive_description(name).map(str::to_owned),
                    supported: true,
                });
            }
            if !cx.quoted {
                for (word, description) in CONTEXTUAL_KEYWORDS.iter().take(1) {
                    c.offer(rank + 1, word, || keyword_item(word, description));
                }
            }
            if position == Position::FieldType {
                expression(&mut c, &full, &cx, env);
            }
        }
        Position::Expression => expression(&mut c, &full, &cx, env),
    }
    let (items, incomplete) = c.finish(max_items);
    Completions {
        span: cx.span,
        prefix: cx.prefix,
        items,
        incomplete,
    }
}

/// The typed text at a cursor.
struct Context {
    span: Span,
    prefix: String,
    /// Inside `#"…`.
    quoted: bool,
    /// Written `@name`.
    inclusive: bool,
}

/// The name being typed at `offset`; `None` in a comment, text or number.
fn context(s: &Scan<'_>, src: &str, offset: usize) -> Option<Context> {
    if s.in_comment(offset) {
        return None;
    }
    for t in &s.tokens {
        let (start, end) = (t.span.start, t.span.end);
        let open = s.unterminated(start);
        let inside = start < offset && (offset < end || (open && offset == end));
        match &t.kind {
            TokenKind::QuotedIdentifier(_) if inside => {
                // `#|"…"` is neither the name nor before it.
                let content = start + 2;
                if offset < content {
                    return None;
                }
                let prefix = src.get(content..offset).unwrap_or_default().to_owned();
                return Some(Context {
                    span: t.span,
                    prefix,
                    quoted: true,
                    inclusive: before(src, start) == Some('@'),
                });
            }
            TokenKind::Text(_) | TokenKind::Verbatim(_) if inside => return None,
            TokenKind::Number(_) if start < offset && offset <= end => return None,
            _ => {}
        }
    }
    let word = |c: char| is_identifier_part(c) || c == '.';
    let mut start = offset;
    while let Some(c) = before(src, start).filter(|c| word(*c)) {
        start -= c.len_utf8();
    }
    // A name starts with a letter or `_` (`1.5x` is a number, then `x`).
    while let Some(c) = src.get(start..offset).and_then(|r| r.chars().next()) {
        if is_identifier_start(c) {
            break;
        }
        start += c.len_utf8();
    }
    if before(src, start) == Some('#') {
        start -= 1;
    }
    let mut end = offset;
    while let Some(c) = src
        .get(end..)
        .and_then(|r| r.chars().next())
        .filter(|c| word(*c))
    {
        end += c.len_utf8();
    }
    Some(Context {
        span: Span::new(start, end),
        prefix: src.get(start..offset).unwrap_or_default().to_owned(),
        quoted: false,
        inclusive: before(src, start) == Some('@'),
    })
}

fn before(src: &str, at: usize) -> Option<char> {
    src.get(..at).and_then(|r| r.chars().next_back())
}

/// The names of an expression position.
fn expression(c: &mut Collector, s: &Scan<'_>, cx: &Context, env: &Environment) {
    let locals = s.visible(cx.span.start, cx.inclusive);
    let mut hidden: HashSet<&str> = HashSet::new();
    for local in &locals {
        hidden.insert(local.name.as_str());
        c.offer(1, &local.name, || local_item(s, local));
    }
    for (i, entry) in env.entries().iter().enumerate() {
        if !hidden.insert(entry.name.as_str()) {
            continue;
        }
        let function = env.function_at(i);
        let (kind, detail) = match (entry.kind, function) {
            (EnvironmentKind::Query, Some(shape)) => (
                NameKind::QueryFunction,
                signature(&crate::refs::identifier(&entry.name), shape).0,
            ),
            (EnvironmentKind::Query, None) => (NameKind::Query, "query".to_owned()),
            (EnvironmentKind::Parameter, _) => (
                NameKind::QueryParameter,
                match &entry.type_name {
                    Some(t) => format!("query parameter ({t})"),
                    None => "query parameter".to_owned(),
                },
            ),
            (EnvironmentKind::Table, _) => (NameKind::Table, "table".to_owned()),
        };
        c.offer(2, &entry.name, || CompletionItem {
            label: entry.name.clone(),
            kind,
            insert_text: crate::refs::identifier(&entry.name),
            detail,
            description: entry.description.clone(),
            supported: true,
        });
    }
    if !cx.quoted {
        for k in Keyword::ALL {
            let name = k.as_str();
            c.offer(3, name, || {
                let mut item = keyword_item(name, keyword_description(k));
                if let Some(shape) = constructor_shape(k) {
                    item.kind = NameKind::Function;
                    item.detail = signature(name, &shape).0;
                    item.supported = crate::stdlib::library().get(name).is_some();
                }
                item
            });
        }
    }
    for entry in catalog() {
        if hidden.contains(entry.name) {
            continue;
        }
        let supported = entry.implemented();
        c.offer(if supported { 3 } else { 4 }, entry.name, || {
            let (kind, detail) = match &entry.kind {
                LibraryKind::Function(_) => (
                    NameKind::Function,
                    entry.signature().unwrap_or_else(|| entry.name.to_owned()),
                ),
                LibraryKind::Type(kind) => (NameKind::Type, format!("type ({kind})")),
                LibraryKind::Constant { kind, .. } | LibraryKind::Value(kind) => {
                    (NameKind::Constant, format!("constant ({kind})"))
                }
            };
            CompletionItem {
                label: entry.name.to_owned(),
                kind,
                insert_text: crate::refs::identifier(entry.name),
                detail,
                description: None,
                supported,
            }
        });
    }
}

fn local_item(s: &Scan<'_>, local: &Local) -> CompletionItem {
    let value = s.value_text(local);
    let insert_text = crate::refs::identifier(&local.name);
    let detail = match local.kind {
        NameKind::Parameter if local.value.is_none() => "parameter".to_owned(),
        NameKind::Parameter => format!("parameter ({})", value.unwrap_or("any")),
        kind => match value.and_then(function_shape) {
            Some(shape) => signature(&insert_text, &shape).0,
            None => match kind {
                NameKind::Member => "section member".to_owned(),
                other => other.as_str().to_owned(),
            },
        },
    };
    CompletionItem {
        label: local.name.clone(),
        kind: local.kind,
        insert_text,
        detail,
        description: None,
        supported: true,
    }
}

fn keyword_item(word: &str, description: &str) -> CompletionItem {
    CompletionItem {
        label: word.to_owned(),
        kind: NameKind::Keyword,
        insert_text: word.to_owned(),
        detail: "keyword".to_owned(),
        description: Some(description.to_owned()),
        supported: true,
    }
}

struct Candidate {
    quality: u8,
    rank: u8,
    key: String,
    item: CompletionItem,
}

struct Collector {
    /// Lower-cased typed text.
    prefix: String,
    out: Vec<Candidate>,
}

impl Collector {
    /// Adds the item `make` builds when `name` matches the prefix; lower `rank`s come first
    /// among equal matches.
    fn offer(&mut self, rank: u8, name: &str, make: impl FnOnce() -> CompletionItem) {
        // `#""` is a valid name, but an empty label is nothing to pick.
        if name.is_empty() {
            return;
        }
        let Some(quality) = quality(name, &self.prefix) else {
            return;
        };
        self.out.push(Candidate {
            quality,
            rank,
            key: name.to_lowercase(),
            item: make(),
        });
    }

    fn finish(mut self, max_items: usize) -> (Vec<CompletionItem>, bool) {
        self.out.sort_by(|a, b| {
            (a.quality, a.rank, &a.key, &a.item.label).cmp(&(
                b.quality,
                b.rank,
                &b.key,
                &b.item.label,
            ))
        });
        // A name shows once, as what it binds to.
        let mut seen = HashSet::new();
        let mut items: Vec<CompletionItem> = self
            .out
            .into_iter()
            .filter(|c| seen.insert(c.item.label.clone()))
            .map(|c| c.item)
            .collect();
        let incomplete = max_items > 0 && items.len() > max_items;
        if incomplete {
            items.truncate(max_items);
        }
        (items, incomplete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::environment::EnvironmentEntry;

    fn at(src: &str, env: &Environment) -> Completions {
        let offset = src.find('|').unwrap_or(src.len());
        complete(&src.replacen('|', "", 1), offset, env, 20)
    }

    fn labels(c: &Completions) -> Vec<&str> {
        c.items.iter().map(|i| i.label.as_str()).collect()
    }

    fn model() -> Environment {
        Environment::with_entries([
            EnvironmentEntry::new("Orders", EnvironmentKind::Table, ""),
            EnvironmentEntry::new("Url", EnvironmentKind::Parameter, "\"x\"")
                .with_type_name(Some("Text".to_owned())),
            EnvironmentEntry::new("Clean", EnvironmentKind::Query, "(t as table) => t"),
        ])
    }

    #[test]
    fn steps_come_first_then_the_model_keywords_and_library() {
        let env = model();
        let c = at(
            "let Source = Orders, #\"Changed Type\" = Source in Cha|",
            &env,
        );
        assert_eq!(c.prefix, "Cha");
        assert_eq!(
            c.items.first().map(|i| i.insert_text.as_str()),
            Some("#\"Changed Type\"")
        );
        let c = at("let Source = 1, Sorted = 2 in So|", &env);
        assert_eq!(labels(&c).get(..2), Some(&["Sorted", "Source"][..]));
        let c = at("let Source = O|", &env);
        assert_eq!(labels(&c).first(), Some(&"Orders"));
        let c = at("Cle|", &env);
        let clean = c.items.first().unwrap();
        assert_eq!(
            (clean.kind, clean.detail.as_str()),
            (NameKind::QueryFunction, "Clean(t as table) as any")
        );
        let c = at("Table.AddCol|umn", &env);
        assert_eq!(c.span, Span::new(0, 15));
        assert_eq!(labels(&c).first(), Some(&"Table.AddColumn"));
        let first = c.items.first().unwrap();
        assert_eq!(
            Some(first.detail.clone()),
            super::super::library::catalog_entry("Table.AddColumn").and_then(|e| e.signature())
        );
        let c = at("#da|", &env);
        assert_eq!(c.prefix, "#da");
        assert_eq!(labels(&c).get(..2), Some(&["#date", "#datetime"][..]));
        let c = at("l|", &env);
        assert_eq!(labels(&c).first(), Some(&"let"));
        // A step hides the query of the same name.
        let c = at("let Orders = 1 in Orde|", &env);
        let orders: Vec<_> = c.items.iter().filter(|i| i.label == "Orders").collect();
        assert_eq!(orders.len(), 1);
        assert_eq!(orders.first().map(|i| i.kind), Some(NameKind::Step));
        // `#""` is a valid step name but never an item.
        let c = at("let #\"\" = 1, Source = 2 in |", &env);
        assert!(labels(&c).contains(&"Source"));
        assert!(c.items.iter().all(|i| !i.label.is_empty()));
    }

    #[test]
    fn scope_follows_the_text() {
        let env = Environment::unknown();
        let has = |src: &str, name: &str| labels(&at(src, &env)).contains(&name);
        assert!(has("Table.AddColumn(t, \"x\", each _|", "_"));
        assert!(!has("Table.AddColumn(t, \"x\", each 1, _|", "_"));
        assert!(has("(value as number) => val|", "value"));
        assert!(has("[a = 1, b = a|]", "a"));
        // Not in its own initializer, except as `@name`.
        assert!(!has("let fact = fac|", "fact"));
        assert!(has("let fact = (n) => @fac|", "fact"));
        let c = at("let fact = (n) => @fa|", &env);
        assert_eq!(c.span, Span::new(19, 21));
    }

    #[test]
    fn quoted_identifiers_complete_inside_the_quotes() {
        let env = model();
        let c = at("let #\"Changed Type\" = 1 in #\"Chan|", &env);
        assert_eq!(c.prefix, "Chan");
        assert_eq!(c.span, Span::new(27, 33));
        assert_eq!(
            c.items.first().map(|i| i.insert_text.as_str()),
            Some("#\"Changed Type\"")
        );
        assert!(c.items.iter().all(|i| i.kind != NameKind::Keyword));
        let c = at("let #\"Changed Type\" = 1 in #\"Chan|ged Type\"", &env);
        assert_eq!(c.span, Span::new(27, 42));
        assert_eq!(labels(&c).first(), Some(&"Changed Type"));
    }

    #[test]
    fn nothing_where_no_name_belongs() {
        let env = model();
        for src in [
            "// Sou|",
            "/* Sou|",
            "/* Sou| */",
            "\"Sou|\"",
            "\"Sou|",
            "12|",
            "let |",
            "let Source = 1, Cha|",
            "each [Amo|",
            "section S; |",
            "#|\"x\"",
        ] {
            assert!(at(src, &env).items.is_empty(), "{src}");
        }
        assert!(!at("\"x\" |", &env).items.is_empty());
        assert!(!at("// x\n|", &env).items.is_empty());
    }

    #[test]
    fn types_after_as() {
        let env = model();
        let c = at("(x as |", &env);
        assert!(
            c.items
                .iter()
                .all(|i| i.kind == NameKind::Type || i.label == "nullable")
        );
        assert_eq!(c.items.len(), PrimitiveType::ALL.len() + 1);
        let c = at("(x as nullable te|", &env);
        assert_eq!(labels(&c).first(), Some(&"text"));
        let c = at("type table [a = num|", &env);
        assert_eq!(labels(&c).first(), Some(&"number"));
        // A field type may be any primary expression too.
        let c = at("type table [a = Int64.Ty|", &env);
        assert_eq!(labels(&c).first(), Some(&"Int64.Type"));
        let c = at("let T = type number, R = type [a = T|", &env);
        assert_eq!(labels(&c).first(), Some(&"T"));
    }

    #[test]
    fn limits_and_every_offset() {
        let env = model();
        let c = complete("", 0, &env, 10);
        assert_eq!(c.items.len(), 10);
        assert!(c.incomplete);
        let all = complete("", 0, &env, 0);
        assert!(all.items.len() > 1000 && !all.incomplete);
        let src = "section S; shared A = let a = (x, optional y as text) => [p = x, q = {1..2}], #\"é b\" = try a(1) otherwise each _ in if #\"é b\" then #date(1, 2, 3) else type table [c = number]; B = \"open /* ";
        // Every cursor of the whole text, and the end of every prefix (as while typing).
        let cursors = (0..=src.len() + 1)
            .map(|offset| (src.len(), offset))
            .chain((0..=src.len()).map(|end| (end, end)));
        for (end, offset) in cursors {
            let Some(text) = src.get(..end) else { continue };
            let c = complete(text, offset, &env, 5);
            assert!(c.span.start <= c.span.end && c.span.end <= text.len());
        }
    }
}
