//! What the editor services say about names: kinds, keywords, primitive types, the `#date`-style
//! constructors, signatures and how a typed prefix matches a name. The descriptions are
//! DashCraft's own words.

use super::environment::{FunctionShape, ParamShape};
use super::library::{LibraryEntry, LibraryKind, identifier};
use crate::lexer::Keyword;

/// What a name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameKind {
    /// A `let` variable of the text.
    Step,
    /// A function parameter of the text (`_` of `each` too).
    Parameter,
    /// A field of a record expression of the text.
    Field,
    /// A member of the section document.
    Member,
    /// Another query of the model.
    Query,
    /// A Power Query parameter of the model.
    QueryParameter,
    /// A query of the model that is a function.
    QueryFunction,
    /// A table of the model loaded by an M query.
    Table,
    /// A standard-library function, or a `#date`-style constructor.
    Function,
    /// A type: a standard-library type constant (`Int64.Type`) or a primitive type (`text`).
    Type,
    /// A standard-library constant or other value (`Number.PI`, `Culture.Current`).
    Constant,
    /// A keyword (`let`, `each`, `#table`, also `nullable` and `optional`).
    Keyword,
}

impl NameKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NameKind::Step => "step",
            NameKind::Parameter => "parameter",
            NameKind::Field => "field",
            NameKind::Member => "member",
            NameKind::Query => "query",
            NameKind::QueryParameter => "queryParameter",
            NameKind::QueryFunction => "queryFunction",
            NameKind::Table => "table",
            NameKind::Function => "function",
            NameKind::Type => "type",
            NameKind::Constant => "constant",
            NameKind::Keyword => "keyword",
        }
    }
}

/// What a keyword does.
pub(crate) fn keyword_description(k: Keyword) -> &'static str {
    match k {
        Keyword::And => {
            "Logical and: true when both operands are true. The right operand is only evaluated when the left one is true."
        }
        Keyword::As => {
            "Asserts that a value has a primitive type (`x as number`), and gives the types of function parameters and results."
        }
        Keyword::Each => {
            "A function of one parameter named `_`: `each [Price] * 2` is `(_) => _[Price] * 2`."
        }
        Keyword::Else => "The value of `if … then … else …` when the condition is false.",
        Keyword::Error => "Raises an error made from a text or an error record.",
        Keyword::False => "The logical value false.",
        Keyword::If => "Chooses between two values: `if condition then value else other`.",
        Keyword::In => "Ends the variables of a `let`; the expression after it is the result.",
        Keyword::Is => "Tests whether a value has a primitive type (`x is text`).",
        Keyword::Let => {
            "Names intermediate values (the steps of a query) and computes the expression after `in` from them."
        }
        Keyword::Meta => {
            "Attaches a metadata record to a value (`\"x\" meta [IsParameterQuery = true]`)."
        }
        Keyword::Not => "Logical negation.",
        Keyword::Null => "The null value: no value.",
        Keyword::Or => {
            "Logical or: true when either operand is true. The right operand is only evaluated when the left one is false."
        }
        Keyword::Otherwise => {
            "The value of `try … otherwise …` when the protected expression raises an error."
        }
        Keyword::Section => {
            "Starts a section document: named members (the queries of a model) ending with `;`."
        }
        Keyword::Shared => "Makes a section member visible outside its section.",
        Keyword::Then => "The value of `if … then … else …` when the condition is true.",
        Keyword::True => "The logical value true.",
        Keyword::Try => {
            "Catches an error: `try x otherwise y`, or a record with `HasError`, `Value` and `Error` fields."
        }
        Keyword::Type => {
            "Starts a type expression (`type table [Name = text]`); as a type, the type of types."
        }
        Keyword::HashBinary => "Makes a binary value from a list of byte values or a base-64 text.",
        Keyword::HashDate => "Makes a date from a year, month and day.",
        Keyword::HashDateTime => {
            "Makes a datetime from a year, month, day, hour, minute and second."
        }
        Keyword::HashDateTimeZone => {
            "Makes a datetimezone from a date, a time and an offset in hours and minutes."
        }
        Keyword::HashDuration => "Makes a duration from days, hours, minutes and seconds.",
        Keyword::HashInfinity => "Positive infinity, a number.",
        Keyword::HashNan => "Not a number (NaN), a number.",
        Keyword::HashSections => "A record of every section document and its members.",
        Keyword::HashShared => {
            "A record of every shared name: the queries of the model and the standard library."
        }
        Keyword::HashTable => {
            "Makes a table from its columns (a list of names, a number or a table type) and a list of rows."
        }
        Keyword::HashTime => "Makes a time from an hour, minute and second.",
    }
}

/// Words that work like keywords but lex as identifiers.
pub(crate) const CONTEXTUAL_KEYWORDS: [(&str, &str); 2] = [
    (
        "nullable",
        "Allows null besides the type that follows (`nullable text`).",
    ),
    (
        "optional",
        "Marks a function parameter that callers may leave out.",
    ),
];

/// The parameters of a `#date`-style constructor, as `name as type`, and its result type.
pub(crate) fn constructor(
    k: Keyword,
) -> Option<(&'static [(&'static str, &'static str)], &'static str)> {
    const N: &str = "number";
    Some(match k {
        Keyword::HashBinary => (&[("value", "any")], "binary"),
        Keyword::HashDate => (&[("year", N), ("month", N), ("day", N)], "date"),
        Keyword::HashDateTime => (
            &[
                ("year", N),
                ("month", N),
                ("day", N),
                ("hour", N),
                ("minute", N),
                ("second", N),
            ],
            "datetime",
        ),
        Keyword::HashDateTimeZone => (
            &[
                ("year", N),
                ("month", N),
                ("day", N),
                ("hour", N),
                ("minute", N),
                ("second", N),
                ("offsetHours", N),
                ("offsetMinutes", N),
            ],
            "datetimezone",
        ),
        Keyword::HashDuration => (
            &[("days", N), ("hours", N), ("minutes", N), ("seconds", N)],
            "duration",
        ),
        Keyword::HashTable => (&[("columns", "any"), ("rows", "list")], "table"),
        Keyword::HashTime => (&[("hour", N), ("minute", N), ("second", N)], "time"),
        _ => return None,
    })
}

/// The shape of a `#date`-style constructor.
pub(crate) fn constructor_shape(k: Keyword) -> Option<FunctionShape> {
    let (params, returns) = constructor(k)?;
    Some(FunctionShape {
        params: params
            .iter()
            .map(|(name, ty)| ParamShape {
                name: (*name).to_owned(),
                optional: false,
                ty: (*ty).to_owned(),
            })
            .collect(),
        returns: returns.to_owned(),
    })
}

/// The shape of a library function.
pub(crate) fn library_shape(entry: &LibraryEntry) -> Option<FunctionShape> {
    let LibraryKind::Function(f) = &entry.kind else {
        return None;
    };
    Some(FunctionShape {
        params: f
            .params
            .iter()
            .map(|p| ParamShape {
                name: p.name.to_owned(),
                optional: p.optional,
                ty: p.ty.to_owned(),
            })
            .collect(),
        returns: f.returns.to_owned(),
    })
}

/// What a primitive type holds.
pub(crate) fn primitive_description(name: &str) -> Option<&'static str> {
    Some(match name {
        "action" => "Actions that change data in a source (accepted by Power Query).",
        "any" => "Every value.",
        "anynonnull" => "Every value except null.",
        "binary" => "Sequences of bytes.",
        "date" => "Dates.",
        "datetime" => "Dates with a time of day.",
        "datetimezone" => "Dates with a time of day and an offset from UTC.",
        "duration" => "Lengths of time.",
        "function" => "Functions.",
        "list" => "Lists of values.",
        "logical" => "true and false.",
        "none" => "No value at all (the result type of a function that always fails).",
        "null" => "Only null.",
        "number" => "Numbers.",
        "record" => "Records: named fields.",
        "table" => "Tables: rows of named columns.",
        "text" => "Texts.",
        "time" => "Times of day.",
        "type" => "Types.",
        _ => return None,
    })
}

/// `name(params) as returns`, with the byte range of each parameter (`name as type`, without
/// `optional`) in it. `name` is written as M (`Table.AddColumn`, `#"My Function"`, `#date`).
pub(crate) fn signature(name: &str, shape: &FunctionShape) -> (String, Vec<(usize, usize)>) {
    let mut out = format!("{name}(");
    let mut labels = Vec::with_capacity(shape.params.len());
    for (i, p) in shape.params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if p.optional {
            out.push_str("optional ");
        }
        let start = out.len();
        out.push_str(&identifier(&p.name));
        out.push_str(" as ");
        out.push_str(&p.ty);
        labels.push((start, out.len()));
    }
    out.push_str(") as ");
    out.push_str(&shape.returns);
    (out, labels)
}

/// How well `candidate` matches the lower-cased `prefix`: 0 the same name, 1 starts with it, 2 a
/// word of it starts with it (after `.`, a space or a lower-to-upper case change), 3 contains
/// it; `None` when it does not match. Every name matches an empty prefix (1).
pub(crate) fn quality(candidate: &str, prefix: &str) -> Option<u8> {
    if prefix.is_empty() {
        return Some(1);
    }
    let lower = candidate.to_lowercase();
    if lower == prefix {
        return Some(0);
    }
    if lower.starts_with(prefix) {
        return Some(1);
    }
    let mut prev: Option<char> = None;
    for (i, ch) in candidate.char_indices() {
        let word_start = match prev {
            None => false,
            Some(p) => !p.is_alphanumeric() || (p.is_lowercase() && ch.is_uppercase()),
        };
        if word_start
            && ch.is_alphanumeric()
            && candidate
                .get(i..)
                .is_some_and(|rest| rest.to_lowercase().starts_with(prefix))
        {
            return Some(2);
        }
        prev = Some(ch);
    }
    lower.contains(prefix).then_some(3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::library::{catalog, catalog_entry};

    #[test]
    fn signatures_match_the_catalog_and_label_parameters() {
        for e in catalog() {
            if let Some(shape) = library_shape(e) {
                let (sig, labels) = signature(e.name, &shape);
                assert_eq!(Some(sig.clone()), e.signature(), "{}", e.name);
                for ((start, end), p) in labels.iter().zip(&shape.params) {
                    let label = sig.get(*start..*end).unwrap_or_default();
                    assert!(label.ends_with(&format!(" as {}", p.ty)), "{label}");
                }
            }
        }
        let shape = constructor_shape(Keyword::HashDate).unwrap();
        let (sig, labels) = signature("#date", &shape);
        assert_eq!(
            sig,
            "#date(year as number, month as number, day as number) as date"
        );
        assert_eq!(sig.get(labels[1].0..labels[1].1), Some("month as number"));
        let upper = library_shape(catalog_entry("Text.Upper").unwrap()).unwrap();
        let (_, labels) = signature("Text.Upper", &upper);
        assert_eq!(labels.len(), 2);
    }

    #[test]
    fn every_keyword_and_primitive_type_is_described() {
        for k in Keyword::ALL {
            assert!(keyword_description(k).ends_with('.'), "{k:?}");
            assert_eq!(
                constructor(k).is_some(),
                matches!(
                    k,
                    Keyword::HashBinary
                        | Keyword::HashDate
                        | Keyword::HashDateTime
                        | Keyword::HashDateTimeZone
                        | Keyword::HashDuration
                        | Keyword::HashTable
                        | Keyword::HashTime
                )
            );
        }
        for p in crate::ast::PrimitiveType::ALL {
            assert!(primitive_description(p.name()).is_some(), "{p:?}");
        }
    }

    #[test]
    fn quality_ranks_exact_prefix_word_and_substring() {
        assert_eq!(quality("Table.AddColumn", "table.addcolumn"), Some(0));
        assert_eq!(quality("Table.AddColumn", "tab"), Some(1));
        assert_eq!(quality("Table.AddColumn", "add"), Some(2));
        assert_eq!(quality("Table.AddColumn", "column"), Some(2));
        assert_eq!(quality("Changed Type", "type"), Some(2));
        assert_eq!(quality("Table.AddColumn", "dcol"), Some(3));
        assert_eq!(quality("Table.AddColumn", "xyz"), None);
        assert_eq!(quality("x", ""), Some(1));
    }
}
