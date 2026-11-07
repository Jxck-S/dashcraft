//! Hover and signature help for M text.

use crate::ast::PrimitiveType;
use crate::lexer::{Keyword, TokenKind};
use crate::span::Span;

use super::diagnostics::unsupported_message;
use super::environment::{Environment, EnvironmentKind, FunctionShape, function_shape};
use super::library::{LibraryKind, catalog_entry};
use super::names::{
    CONTEXTUAL_KEYWORDS, NameKind, constructor_shape, keyword_description, library_shape,
    primitive_description, signature,
};
use super::scan::{Local, Role, Scan, scan, token_name};

/// Longest expression a hover shows, in bytes.
const MAX_HOVER_EXPRESSION: usize = 2000;

/// A parameter of a [`SignatureHelp`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureParameter {
    pub name: String,
    /// `nullable text`; `any` when not given.
    pub ty: String,
    pub optional: bool,
    /// Where `name as type` is in [`SignatureHelp::label`] (bytes).
    pub label: Span,
}

/// The signature of the function called at a cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHelp {
    /// The called name as M writes it (`Table.AddColumn`, `#date`, `#"My Function"`).
    pub name: String,
    /// The called name in the text.
    pub name_span: Span,
    /// [`NameKind::Function`] for the library and the `#date`-style constructors; otherwise the
    /// kind of the step, parameter, field, member or query that holds the function.
    pub kind: NameKind,
    /// `Text.Upper(text as nullable text, optional culture as nullable text) as nullable text`.
    pub label: String,
    pub parameters: Vec<SignatureParameter>,
    /// The return type.
    pub returns: String,
    /// The 0-based argument the cursor is in.
    pub argument: usize,
    /// [`Self::argument`] when the function has such a parameter.
    pub active_parameter: Option<usize>,
    /// Whether DashCraft evaluates the function (always for functions of the text and model).
    pub supported: bool,
}

/// The signature of the innermost function call whose argument list contains `offset` (a byte
/// offset): a library function, a `#date`-style constructor, a function step of the text or a
/// function query of `env`. `None` outside a call, in a comment, or when the called name is not
/// a known function.
#[must_use]
pub fn signature_help(src: &str, offset: usize, env: &Environment) -> Option<SignatureHelp> {
    let offset = floor_char_boundary(src, offset);
    let s = scan(src, Some(offset));
    if s.in_comment(offset) {
        return None;
    }
    let (call_index, argument) = s.at_cursor?.call?;
    let call = s.calls.get(call_index)?;
    let (name, kind, shape, supported) = callee(&s, call.token, &call.name, env)?;
    let (label, ranges) = signature(&name, &shape);
    let parameters = shape
        .params
        .into_iter()
        .zip(ranges)
        .map(|(p, (start, end))| SignatureParameter {
            name: p.name,
            ty: p.ty,
            optional: p.optional,
            label: Span::new(start, end),
        })
        .collect::<Vec<_>>();
    let active_parameter = (argument < parameters.len()).then_some(argument);
    Some(SignatureHelp {
        name,
        name_span: call.name_span,
        kind,
        label,
        parameters,
        returns: shape.returns,
        argument,
        active_parameter,
        supported,
    })
}

/// The function `name` (token `token`) refers to: its name as M, kind, shape and support.
fn callee(
    s: &Scan<'_>,
    token: usize,
    name: &str,
    env: &Environment,
) -> Option<(String, NameKind, FunctionShape, bool)> {
    let at = s.tokens.get(token)?.span.start;
    if let Some(local) = s.resolve(name, at, inclusive(s, token)) {
        // A step, parameter or field hides the model and the library.
        let shape = s.value_text(local).and_then(function_shape)?;
        return Some((crate::refs::identifier(name), local.kind, shape, true));
    }
    if let Some(entry) = env.entry(name) {
        let shape = env.function(name)?.clone();
        let kind = match entry.kind {
            EnvironmentKind::Query => NameKind::QueryFunction,
            EnvironmentKind::Parameter | EnvironmentKind::Table => return None,
        };
        return Some((crate::refs::identifier(name), kind, shape, true));
    }
    if let Some(entry) = catalog_entry(name) {
        let shape = library_shape(entry)?;
        return Some((
            entry.name.to_owned(),
            NameKind::Function,
            shape,
            entry.implemented(),
        ));
    }
    let k = Keyword::lookup(name)?;
    let shape = constructor_shape(k)?;
    Some((
        name.to_owned(),
        NameKind::Function,
        shape,
        crate::stdlib::library().get(name).is_some(),
    ))
}

/// Whether the name at `token` is written `@name` (which reaches its own initializer).
fn inclusive(s: &Scan<'_>, token: usize) -> bool {
    token
        .checked_sub(1)
        .and_then(|p| s.tokens.get(p))
        .is_some_and(|t| {
            t.kind == TokenKind::At
                && s.tokens
                    .get(token)
                    .is_some_and(|n| t.span.end == n.span.start)
        })
}

/// Information about the name under a cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// The hovered token.
    pub span: Span,
    pub kind: NameKind,
    /// The name as M writes it (`Changed Type` is `#"Changed Type"`).
    pub name: String,
    /// A function's signature.
    pub signature: Option<String>,
    /// A parameter's type, a type constant's or constant's primitive kind, a query parameter's
    /// type (`Text`).
    pub type_name: Option<String>,
    /// An optional function parameter.
    pub optional: bool,
    /// A library constant's value as an M literal.
    pub value: Option<String>,
    /// The M text of a step, field, member or query (shortened when long).
    pub expression: Option<String>,
    /// What a keyword or primitive type is, or the model's description of a query.
    pub description: Option<String>,
    /// Whether DashCraft evaluates a library name; `false` with [`Self::unsupported`] set.
    pub supported: bool,
    /// Why DashCraft cannot evaluate the name.
    pub unsupported: Option<String>,
}

impl Hover {
    fn new(span: Span, kind: NameKind, name: String) -> Self {
        Self {
            span,
            kind,
            name,
            signature: None,
            type_name: None,
            optional: false,
            value: None,
            expression: None,
            description: None,
            supported: true,
            unsupported: None,
        }
    }

    /// The hover as Markdown (code in `powerquery` fences).
    #[must_use]
    pub fn markdown(&self) -> String {
        let label = match self.kind {
            NameKind::Step => "step",
            NameKind::Parameter if self.optional => "optional parameter",
            NameKind::Parameter => "parameter",
            NameKind::Field => "field",
            NameKind::Member => "section member",
            NameKind::Query => "query",
            NameKind::QueryParameter => "query parameter",
            NameKind::QueryFunction => "query function",
            NameKind::Table => "table",
            NameKind::Function => "function",
            NameKind::Type => "type",
            NameKind::Constant => "constant",
            NameKind::Keyword => "keyword",
        };
        let mut out = format!("`{}`: {label}", self.name);
        if let Some(t) = &self.type_name {
            out.push_str(&format!(" ({t})"));
        }
        let library = matches!(
            self.kind,
            NameKind::Function | NameKind::Type | NameKind::Constant
        );
        if let (Some(sig), true) = (&self.signature, library) {
            out.push_str(&format!("\n\n```powerquery\n{sig}\n```"));
        }
        if let Some(v) = &self.value {
            out.push_str(&format!("\n\nValue: `{v}`"));
        }
        if let Some(d) = &self.description {
            out.push_str(&format!("\n\n{d}"));
        }
        if let Some(u) = &self.unsupported {
            out.push_str(&format!("\n\n{u}"));
        }
        if let Some(e) = &self.expression {
            out.push_str(&format!("\n\n```powerquery\n{e}\n```"));
        }
        out
    }
}

/// Information about the name or keyword at `offset` (a byte offset): the token that contains
/// it, or ends at it. Names resolve as Power BI Desktop binds them: the steps, parameters and
/// fields of the text, then the queries of `env`, then the standard library. `None` for field
/// names, unknown names and anything that is not a name.
#[must_use]
pub fn hover(src: &str, offset: usize, env: &Environment) -> Option<Hover> {
    let offset = floor_char_boundary(src, offset);
    let s = scan(src, None);
    let hoverable = |k: &TokenKind| {
        matches!(
            k,
            TokenKind::Identifier(_) | TokenKind::QuotedIdentifier(_) | TokenKind::Keyword(_)
        )
    };
    let i = s
        .tokens
        .iter()
        .position(|t| t.span.start <= offset && offset < t.span.end && hoverable(&t.kind))
        .or_else(|| {
            s.tokens
                .iter()
                .position(|t| t.span.end == offset && hoverable(&t.kind))
        })?;
    let token = s.tokens.get(i)?;
    let span = token.span;
    match (s.roles.get(i)?, &token.kind) {
        (Role::Declaration(l), _) => Some(local_hover(&s, span, s.locals.get(*l)?)),
        (Role::FieldName | Role::SectionName, _) => None,
        (_, TokenKind::Keyword(k)) => Some(keyword_hover(span, *k)),
        (_, kind) => {
            let name = token_name(kind)?;
            if let Some(local) = s.resolve(name, span.start, inclusive(&s, i)) {
                return Some(local_hover(&s, span, local));
            }
            let prev = i
                .checked_sub(1)
                .and_then(|p| s.tokens.get(p))
                .map(|t| &t.kind);
            let type_word = match prev {
                Some(TokenKind::Keyword(Keyword::As | Keyword::Is | Keyword::Type)) => true,
                Some(TokenKind::Identifier(n)) => n == "nullable",
                _ => false,
            };
            if type_word && let Some(h) = primitive_hover(span, name) {
                return Some(h);
            }
            if let Some(h) = environment_hover(span, name, env) {
                return Some(h);
            }
            if let Some(h) = library_hover(span, name) {
                return Some(h);
            }
            if let Some((word, description)) = CONTEXTUAL_KEYWORDS.iter().find(|(w, _)| *w == name)
            {
                let mut h = Hover::new(span, NameKind::Keyword, (*word).to_owned());
                h.description = Some((*description).to_owned());
                return Some(h);
            }
            primitive_hover(span, name)
        }
    }
}

fn local_hover(s: &Scan<'_>, span: Span, local: &Local) -> Hover {
    let mut h = Hover::new(span, local.kind, crate::refs::identifier(&local.name));
    let value = s.value_text(local);
    if local.kind == NameKind::Parameter {
        h.optional = local.optional;
        h.type_name = Some(value.unwrap_or("any").to_owned());
        if local.name == "_"
            && local.value.is_none()
            && s.src.get(local.decl.start..local.decl.end) == Some("each")
        {
            h.type_name = None;
            h.description = Some("The parameter of `each`.".to_owned());
        }
    } else if let Some(v) = value {
        h.signature = function_shape(v).map(|shape| signature(&h.name, &shape).0);
        h.expression = Some(shorten(v));
    }
    h
}

fn environment_hover(span: Span, name: &str, env: &Environment) -> Option<Hover> {
    let entry = env.entry(name)?;
    let function = env.function(name);
    let kind = match entry.kind {
        EnvironmentKind::Query if function.is_some() => NameKind::QueryFunction,
        EnvironmentKind::Query => NameKind::Query,
        EnvironmentKind::Parameter => NameKind::QueryParameter,
        EnvironmentKind::Table => NameKind::Table,
    };
    let mut h = Hover::new(span, kind, crate::refs::identifier(name));
    h.signature = function.map(|shape| signature(&h.name, shape).0);
    h.type_name = entry.type_name.clone();
    h.description = entry.description.clone();
    let text = entry.expression.trim();
    if !text.is_empty() {
        h.expression = Some(shorten(text));
    }
    Some(h)
}

fn library_hover(span: Span, name: &str) -> Option<Hover> {
    let entry = catalog_entry(name)?;
    let mut h = match &entry.kind {
        LibraryKind::Function(_) => {
            let mut h = Hover::new(span, NameKind::Function, entry.name.to_owned());
            h.signature = entry.signature();
            h
        }
        LibraryKind::Type(kind) => {
            let mut h = Hover::new(span, NameKind::Type, entry.name.to_owned());
            h.type_name = Some((*kind).to_owned());
            h
        }
        LibraryKind::Constant { kind, value } => {
            let mut h = Hover::new(span, NameKind::Constant, entry.name.to_owned());
            h.type_name = Some((*kind).to_owned());
            h.value = Some((*value).to_owned());
            h
        }
        LibraryKind::Value(kind) => {
            let mut h = Hover::new(span, NameKind::Constant, entry.name.to_owned());
            h.type_name = Some((*kind).to_owned());
            h
        }
    };
    if !entry.implemented() {
        h.supported = false;
        h.unsupported = Some(unsupported_message(entry.name));
    }
    Some(h)
}

fn keyword_hover(span: Span, k: Keyword) -> Hover {
    let name = k.as_str();
    let mut h = match constructor_shape(k) {
        Some(shape) => {
            let mut h = Hover::new(span, NameKind::Function, name.to_owned());
            h.signature = Some(signature(name, &shape).0);
            if crate::stdlib::library().get(name).is_none() {
                h.supported = false;
                h.unsupported = Some(unsupported_message(name));
            }
            h
        }
        None => Hover::new(span, NameKind::Keyword, name.to_owned()),
    };
    h.description = Some(keyword_description(k).to_owned());
    h
}

fn primitive_hover(span: Span, name: &str) -> Option<Hover> {
    let p = PrimitiveType::from_name(name)?;
    let mut h = Hover::new(span, NameKind::Type, p.name().to_owned());
    h.description = primitive_description(p.name()).map(str::to_owned);
    Some(h)
}

/// `text`, cut at [`MAX_HOVER_EXPRESSION`] bytes with `…`.
fn shorten(text: &str) -> String {
    if text.len() <= MAX_HOVER_EXPRESSION {
        return text.to_owned();
    }
    let end = floor_char_boundary(text, MAX_HOVER_EXPRESSION);
    format!("{}…", text.get(..end).unwrap_or_default())
}

pub(crate) fn floor_char_boundary(src: &str, offset: usize) -> usize {
    let mut at = offset.min(src.len());
    while !src.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::environment::EnvironmentEntry;

    fn cursor(src: &str) -> (String, usize) {
        let offset = src.find('|').unwrap_or(src.len());
        (src.replacen('|', "", 1), offset)
    }

    fn model() -> Environment {
        Environment::with_entries([
            EnvironmentEntry::new(
                "Orders",
                EnvironmentKind::Table,
                "let Source = Csv.Document(\"x\") in Source",
            )
            .with_description(Some("Order lines.".to_owned())),
            EnvironmentEntry::new(
                "Url",
                EnvironmentKind::Parameter,
                "\"https://example.com\" meta [IsParameterQuery = true, Type = \"Text\"]",
            )
            .with_type_name(Some("Text".to_owned())),
            EnvironmentEntry::new(
                "Clean",
                EnvironmentKind::Query,
                "(t as table, optional n as nullable number) as table => t",
            ),
        ])
    }

    #[test]
    fn signature_help_for_library_constructors_steps_and_queries() {
        let env = model();
        let help = |src: &str| {
            let (text, offset) = cursor(src);
            signature_help(&text, offset, &env)
                .map(|h| (h.label, h.argument, h.active_parameter, h.kind, h.supported))
        };
        let (label, argument, active, kind, _) =
            help("Table.AddColumn(Source, \"x\", each Text.Upper([A]), |").unwrap();
        assert_eq!(
            label,
            catalog_entry("Table.AddColumn")
                .and_then(|e| e.signature())
                .unwrap()
        );
        assert_eq!((argument, active, kind), (3, Some(3), NameKind::Function));
        assert_eq!(
            help("#date(2024, |"),
            Some((
                "#date(year as number, month as number, day as number) as date".to_owned(),
                1,
                Some(1),
                NameKind::Function,
                true
            ))
        );
        assert_eq!(
            help("let F = (x as text, optional y) => x in F(\"a\", |"),
            Some((
                "F(x as text, optional y as any) as any".to_owned(),
                1,
                Some(1),
                NameKind::Step,
                true
            ))
        );
        assert_eq!(
            help("Clean(Orders, 1, |)"),
            Some((
                "Clean(t as table, optional n as nullable number) as table".to_owned(),
                2,
                None,
                NameKind::QueryFunction,
                true
            ))
        );
        // A step that is not a function, a parameter and an unknown name have no signature.
        assert_eq!(help("let Text.Upper = 1 in Text.Upper(|"), None);
        assert_eq!(help("Url(|"), None);
        assert_eq!(help("Nope(|"), None);
        assert_eq!(help("Text.Upper(1) |"), None);
        assert_eq!(help("Text.Upper(/* | */"), None);
        let (text, offset) = cursor("Text.Upper(\"é\", |)");
        let h = signature_help(&text, offset, &env).unwrap();
        assert_eq!(h.name_span, Span::new(0, 10));
        let culture = h.parameters.get(1).unwrap();
        assert_eq!(
            h.label.get(culture.label.start..culture.label.end),
            Some("culture as nullable text")
        );
        assert!(culture.optional);
    }

    #[test]
    fn hover_resolves_like_desktop() {
        let env = model();
        let src = "let Source = Orders, #\"Up per\" = Table.TransformColumns(Source, {{\"A\", each Text.Upper(_)}}), n = Number.PI, t = Int64.Type, f = (x as number, optional y) => x + y in #date(2024, 1, 1) + Url";
        let at = |needle: &str, nth: usize| {
            let offset = src.match_indices(needle).nth(nth).map_or(0, |(i, _)| i);
            hover(src, offset + 1, &env)
        };
        let h = at("Source", 0).unwrap();
        assert_eq!(
            (h.kind, h.expression.as_deref()),
            (NameKind::Step, Some("Orders"))
        );
        let h = at("Source", 1).unwrap();
        assert_eq!(h.kind, NameKind::Step);
        let h = at("Orders", 0).unwrap();
        assert_eq!(h.kind, NameKind::Table);
        assert!(h.markdown().contains("Order lines."));
        let h = at("Up per", 0).unwrap();
        assert_eq!(h.name, "#\"Up per\"");
        let h = at("Table.TransformColumns", 0).unwrap();
        assert_eq!(h.kind, NameKind::Function);
        assert!(h.markdown().starts_with(
            "`Table.TransformColumns`: function\n\n```powerquery\nTable.TransformColumns("
        ));
        let h = at("_", 0).unwrap();
        assert_eq!(h.kind, NameKind::Parameter);
        let h = at("each", 0).unwrap();
        assert_eq!(h.kind, NameKind::Keyword);
        let h = at("Number.PI", 0).unwrap();
        assert_eq!((h.kind, h.value.is_some()), (NameKind::Constant, true));
        let h = at("Int64.Type", 0).unwrap();
        assert_eq!(
            (h.kind, h.type_name.as_deref()),
            (NameKind::Type, Some("number"))
        );
        let h = at("number", 0).unwrap();
        assert_eq!(h.kind, NameKind::Type);
        let h = at("optional", 0).unwrap();
        assert_eq!(h.kind, NameKind::Keyword);
        let h = at("y", 1).unwrap();
        assert_eq!((h.kind, h.optional), (NameKind::Parameter, true));
        let h = at("#date", 0).unwrap();
        assert_eq!(h.kind, NameKind::Function);
        let h = at("Url", 0).unwrap();
        assert_eq!(
            (h.kind, h.type_name.as_deref()),
            (NameKind::QueryParameter, Some("Text"))
        );
        // Field names and unknown names have no hover.
        assert_eq!(hover("each [Amount]", 7, &env), None);
        assert_eq!(hover("Nope", 1, &env), None);
        // At the end of a name.
        assert_eq!(
            hover("Orders", 6, &env).map(|h| h.kind),
            Some(NameKind::Table)
        );
    }

    #[test]
    fn unsupported_library_names_say_so() {
        let unsupported = super::super::library::catalog()
            .iter()
            .find(|e| matches!(e.kind, LibraryKind::Function(_)) && !e.implemented())
            .unwrap();
        let h = hover(unsupported.name, 0, &Environment::unknown()).unwrap();
        assert!(!h.supported);
        assert!(h.markdown().contains("DashCraft does not evaluate"));
    }

    #[test]
    fn every_offset_of_messy_text_answers_without_panicking() {
        let env = model();
        let src = "section S; shared A = let a = (x, optional y as text) => [p = x, q = {1..2}], #\"é b\" = try a(1) otherwise each _ in if #\"é b\" then #date(1, 2, 3) else type table [c = number]; B = \"open /* ";
        for end in (0..=src.len()).filter(|i| src.is_char_boundary(*i)) {
            let text = &src[..end];
            for offset in 0..=end + 1 {
                let _ = hover(text, offset, &env);
                let _ = signature_help(text, offset, &env);
            }
        }
    }
}
