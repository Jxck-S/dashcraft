//! Power Query M language support: lexer, parser, AST, pretty-printer and a lazy evaluator with
//! a standard-library subset.
//!
//! # Evaluation
//!
//! * [`Engine`] evaluates queries with [`EvalOptions`]: a [`BlobResolver`] for
//!   `AzureStorage.Blobs` (offline: [`MapBlobResolver`] over `blob-map.json`), a [`Clock`],
//!   resource [`Limits`] and a cancellation flag.
//! * [`Engine::evaluate_query`] returns a resolved [`Value`];
//!   [`Engine::evaluate_partition_to_arrow`] returns an Arrow `RecordBatch` with the model's
//!   columns (see [`eval::arrow`]).
//! * Errors are [`MError`]s: syntax errors with line/column, M errors (`try`-catchable),
//!   [`MError::Unsupported`] for functions DashCraft does not evaluate (never files, databases or
//!   the network), resolver failures, limits and cancellation.
//! * [`library_names`] lists the implemented library functions and constants.
//!
//! See [`eval`] for semantics notes and deviations from Power Query.
//!
//! ```
//! use pbi_m::{Engine, Value};
//!
//! let v = Engine::default()
//!     .evaluate(r#"Table.RowCount(Table.SelectRows(#table({"a"}, {{1}, {2}}), each [a] > 1))"#)
//!     .unwrap();
//! assert_eq!(v, Value::Number(1.0));
//! ```
//!
//! # Parsing entry points
//!
//! * [`parse_expression`] — an expression document (query body, TMDL partition `source`,
//!   TMDL `expression` such as `"https://..." meta [IsParameterQuery = true]`).
//! * [`parse_document`] / [`parse_document_with_comments`] — an expression *or* a section
//!   document (`section S; shared x = ...;`, as in `.pq` files), optionally with comments.
//! * [`tokenize`] — the token stream and comments (e.g. for syntax highlighting).
//! * [`print_expr`] / [`print_document`] / `Display` — render an AST back to M such that
//!   `parse(print(parse(x))) == parse(x)` modulo spans ([`Expr::clear_spans`]).
//!
//! Errors are [`ParseError`]s with a byte [`Span`], an `expected` list and
//! [`ParseError::render`] / [`line_col`] for `line:column` reporting. No input panics; nesting
//! is limited to [`MAX_NESTING`] levels and tree depth to [`MAX_AST_DEPTH`].
//!
//! # Notes for consumers (evaluator, folding, editors)
//!
//! * [`ExprKind::Parenthesized`] is kept for printing fidelity; it is semantically transparent
//!   (see [`Expr::unparenthesized`]).
//! * `each body` is [`ExprKind::Each`]: a one-parameter function whose parameter is `_`.
//! * Implicit field access `[a]` / projection `[[a]]` have `target: None`, meaning `_[a]`.
//! * `@x` is an identifier with `inclusive: true` (the binding being defined is in scope).
//! * `#date`, `#table`, `#infinity`, `#shared`, ... are [`ast::Intrinsic`]s, not identifiers;
//!   a quoted `#"#date"` is an ordinary identifier.
//! * Dotted names (`Table.SelectRows`, `QuoteStyle.Csv`, `Int64.Type`) are single identifiers.
//! * `and`/`or` short-circuit; `??` evaluates its right operand only for `null`; `meta` is a
//!   [`ast::BinaryOp`]; `x as T` / `x is T` are [`ExprKind::As`] / [`ExprKind::Is`] with a
//!   nullable primitive type.
//! * Number literals keep their source spelling (`NumberLiteral::raw`) and an `f64` value; there
//!   are no negative literals (`-1` is unary minus).
//! * Tree walks recurse once per tree level; [`MAX_AST_DEPTH`] bounds that. Unoptimized builds
//!   need roughly 2 KB of stack per level for a typical recursive walk, so evaluate on a thread
//!   with a generous stack if the tree may be deep.
//!
//! ```
//! use pbi_m::{ast::ExprKind, parse_expression, print_expr};
//!
//! let expr = parse_expression(r#"let Source = #table({"a"}, {{1}}) in Source{0}[a]"#).unwrap();
//! assert!(matches!(expr.kind, ExprKind::Let { .. }));
//! let mut reparsed = parse_expression(&print_expr(&expr)).unwrap();
//! let mut original = expr.clone();
//! reparsed.clear_spans();
//! original.clear_spans();
//! assert_eq!(reparsed, original);
//! ```

pub mod ast;
pub mod connectors;
pub mod error;
pub mod eval;
pub mod lexer;
pub mod merror;
pub mod parser;
pub mod printer;
pub mod span;
pub mod stdlib;
pub mod value;

pub use ast::{Document, Expr, ExprKind, Name, TypeExpr, TypeKind};
pub use error::ParseError;
pub use lexer::{Comment, CommentKind, Lexed, Token, TokenKind, tokenize};
pub use parser::{
    MAX_AST_DEPTH, MAX_NESTING, ParsedDocument, parse_document, parse_document_with_comments,
    parse_expression,
};
pub use printer::{print_document, print_expr, print_type};
pub use span::{Span, line_col};

pub use connectors::{BlobEntry, BlobResolver, MapBlobResolver, NoBlobResolver};
pub use eval::arrow::table_to_record_batch;
pub use eval::{Clock, Engine, EvalOptions, FixedClock, Limits, SystemClock};
pub use merror::{LimitKind, Location, MError, ResolverError, ResolverErrorKind};
pub use stdlib::library_names;
pub use value::{
    BinarySource, BinaryValue, FunctionValue, Lazy, ListValue, RecordValue, TableValue, TypeValue,
    Value, ValueKind,
};
