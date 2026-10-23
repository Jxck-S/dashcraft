//! DAX syntax: lexer, spanned AST, parser, pretty-printer and visitors.
//!
//! ```
//! use pbi_dax::syntax::{parse_expression, print_expr, references, Reference};
//!
//! let src = "CALCULATE(COUNTROWS(Orders), Orders[Status] = \"Completed\") + 0";
//! let expr = parse_expression(src).unwrap();
//! assert_eq!(expr.text(src), src);
//! assert_eq!(print_expr(&expr), src);
//! assert!(references(&expr).contains(&Reference::Table("Orders".into())));
//! ```

pub mod ast;
pub mod error;
pub mod lexer;
pub mod parser;
pub mod printer;
pub mod visit;

pub use ast::{
    BinaryOp, Define, Evaluate, Expr, ExprKind, FunctionDef, FunctionParam, Number, Query, SortDir,
    Span, UnaryOp, VarBinding, prec,
};
pub use error::{ParseError, line_col};
pub use lexer::{Keyword, LexError, Token, TokenKind, is_enum_keyword, tokenize};
pub use parser::{MAX_DEPTH, parse_expression, parse_function_definition, parse_query};
pub use printer::{PrintOptions, pretty_expr, pretty_query, print_expr, print_query};
pub use visit::{
    Reference, Visitor, for_each_expr, reference_sites, referenced_tables, references, walk_expr,
};
