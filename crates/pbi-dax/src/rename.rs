//! Token-level rewriting of DAX references for renames.
//!
//! [`rename_references`] parses an expression, finds the table, column and
//! measure references a rename affects ([`reference_sites`]) and replaces only
//! the name tokens inside them, so whitespace, comments, casing of everything
//! else and line breaks stay exactly as written. Variables in scope, string
//! literals and comments are never rewritten.

use crate::eval::is_supported_function;
use crate::syntax::{
    Keyword, ParseError, Reference, Span, TokenKind, is_enum_keyword, parse_expression,
    reference_sites, tokenize,
};

/// A rename whose references should be rewritten. Names compare
/// case-insensitively, like DAX.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameChange<'a> {
    /// A table: `Old`, `'Old'` and the qualifiers of `Old[Column]`.
    Table { old: &'a str, new: &'a str },
    /// A column of `table`: `table[old]`, and the unqualified `[old]` when
    /// `bare` is set (the caller decides whether `[old]` can only mean this
    /// column in the expression's context).
    Column {
        table: &'a str,
        old: &'a str,
        new: &'a str,
        bare: bool,
    },
    /// A measure of `table`: `[old]` when `bare` is set, and `table[old]`.
    Measure {
        table: &'a str,
        old: &'a str,
        new: &'a str,
        bare: bool,
    },
}

/// Rewrites the references `change` affects in `src`.
///
/// Returns `Ok(None)` when no reference changes, `Ok(Some(text))` with the
/// rewritten expression otherwise.
///
/// # Errors
/// The expression does not parse; nothing is rewritten.
pub fn rename_references(src: &str, change: &NameChange<'_>) -> Result<Option<String>, ParseError> {
    let expr = parse_expression(src)?;
    let mut edits: Vec<(Span, String)> = Vec::new();
    for (reference, span) in reference_sites(&expr) {
        match (&reference, change) {
            (Reference::Table(name), NameChange::Table { old, new }) if same(name, old) => {
                edit_name(src, span, NamePart::Table, new, &mut edits);
            }
            (Reference::Column { table, .. }, NameChange::Table { old, new })
                if same(table, old) =>
            {
                edit_name(src, span, NamePart::Table, new, &mut edits);
            }
            (
                Reference::Column { table, column },
                NameChange::Column {
                    table: owner,
                    old,
                    new,
                    ..
                }
                | NameChange::Measure {
                    table: owner,
                    old,
                    new,
                    ..
                },
            ) if same(table, owner) && same(column, old) => {
                edit_name(src, span, NamePart::Bracket, new, &mut edits);
            }
            (
                Reference::Measure(name),
                NameChange::Column {
                    old,
                    new,
                    bare: true,
                    ..
                }
                | NameChange::Measure {
                    old,
                    new,
                    bare: true,
                    ..
                },
            ) if same(name, old) => {
                edit_name(src, span, NamePart::Bracket, new, &mut edits);
            }
            _ => {}
        }
    }
    if edits.is_empty() {
        return Ok(None);
    }
    edits.sort_by_key(|(span, _)| span.start);
    let mut out = String::with_capacity(src.len());
    let mut at = 0;
    for (span, text) in edits {
        if span.start < at {
            continue;
        }
        out.push_str(src.get(at..span.start).unwrap_or(""));
        out.push_str(&text);
        at = span.end;
    }
    out.push_str(src.get(at..).unwrap_or(""));
    Ok((out != src).then_some(out))
}

/// Whether `name` may be written without quotes as a table name.
#[must_use]
pub fn is_plain_table_name(name: &str) -> bool {
    let mut chars = name.chars();
    let starts_well = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    starts_well
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && Keyword::from_ident(name).is_none()
        && !is_enum_keyword(name)
        && !is_supported_function(name)
}

/// `'name'` with quotes doubled, or `name` when it needs no quotes.
#[must_use]
pub fn quote_table_name(name: &str, prefer_plain: bool) -> String {
    if prefer_plain && is_plain_table_name(name) {
        name.to_owned()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// `[name]` with closing brackets doubled.
#[must_use]
pub fn bracket_name(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

fn same(a: &str, b: &str) -> bool {
    a == b || a.to_lowercase() == b.to_lowercase()
}

#[derive(Clone, Copy)]
enum NamePart {
    /// The first name token of the reference (`Orders` in `Orders[Amount]`).
    Table,
    /// The bracketed token (`[Amount]`).
    Bracket,
}

/// Queues the replacement of one name token inside the reference at `span`.
fn edit_name(src: &str, span: Span, part: NamePart, new: &str, edits: &mut Vec<(Span, String)>) {
    let text = span.text(src);
    let tokens = tokenize(text);
    let mut names = tokens.iter().filter(|t| !t.is_trivia());
    let token = match part {
        NamePart::Table => names.next(),
        NamePart::Bracket => names.rfind(|t| t.kind == TokenKind::BracketIdent),
    };
    let Some(token) = token else {
        return;
    };
    let replacement = match (part, token.kind) {
        (NamePart::Table, TokenKind::Ident) => quote_table_name(new, true),
        (NamePart::Table, TokenKind::QuotedIdent) => quote_table_name(new, false),
        (NamePart::Bracket, TokenKind::BracketIdent) => bracket_name(new),
        _ => return,
    };
    let (Some(start), Some(end)) = (
        span.start.checked_add(token.span.start),
        span.start.checked_add(token.span.end),
    ) else {
        return;
    };
    edits.push((Span::new(start, end), replacement));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(src: &str, old: &str, new: &str) -> Option<String> {
        rename_references(src, &NameChange::Table { old, new }).unwrap()
    }

    fn column(src: &str, bare: bool) -> Option<String> {
        rename_references(
            src,
            &NameChange::Column {
                table: "Orders",
                old: "Line Amount",
                new: "Net Amount",
                bare,
            },
        )
        .unwrap()
    }

    #[test]
    fn table_rename_keeps_layout_and_quoting_style() {
        let src = "CALCULATE ( COUNTROWS ( Orders ), Orders[Status] = \"Orders\" ) // Orders";
        assert_eq!(
            table(src, "orders", "Sales").as_deref(),
            Some("CALCULATE ( COUNTROWS ( Sales ), Sales[Status] = \"Orders\" ) // Orders")
        );
        assert_eq!(
            table("SUM('Orders'[Qty])", "Orders", "Sales").as_deref(),
            Some("SUM('Sales'[Qty])")
        );
        assert_eq!(
            table("COUNTROWS(Orders)", "Orders", "Web Orders").as_deref(),
            Some("COUNTROWS('Web Orders')")
        );
        assert_eq!(
            table("COUNTROWS('A')", "A", "Bob's").as_deref(),
            Some("COUNTROWS('Bob''s')")
        );
        assert_eq!(table("COUNTROWS(Orders)", "Customers", "X"), None);
    }

    #[test]
    fn table_rename_skips_variables_and_quotes_function_names() {
        let src = "VAR Orders = 1 RETURN Orders + COUNTROWS('Orders')";
        assert_eq!(
            table(src, "Orders", "Sales").as_deref(),
            Some("VAR Orders = 1 RETURN Orders + COUNTROWS('Sales')")
        );
        assert_eq!(
            table("COUNTROWS(Orders)", "Orders", "Date").as_deref(),
            Some("COUNTROWS('Date')")
        );
    }

    #[test]
    fn column_rename_qualified_and_bare() {
        let src = "SUMX(Orders, Orders[Line Amount] * 2) + SUMX(Orders, [Line Amount])";
        assert_eq!(
            column(src, false).as_deref(),
            Some("SUMX(Orders, Orders[Net Amount] * 2) + SUMX(Orders, [Line Amount])")
        );
        assert_eq!(
            column(src, true).as_deref(),
            Some("SUMX(Orders, Orders[Net Amount] * 2) + SUMX(Orders, [Net Amount])")
        );
        assert_eq!(column("SUM(Returns[Line Amount])", true), None);
    }

    #[test]
    fn measure_rename_escapes_brackets() {
        let change = NameChange::Measure {
            table: "Orders",
            old: "Total Sales",
            new: "Sales [USD]",
            bare: true,
        };
        assert_eq!(
            rename_references("[Total Sales] / 'Orders'[total sales]", &change)
                .unwrap()
                .as_deref(),
            Some("[Sales [USD]]] / 'Orders'[Sales [USD]]]")
        );
    }

    #[test]
    fn unparsable_expressions_are_errors() {
        assert!(
            rename_references(
                "SUM(Orders[",
                &NameChange::Table {
                    old: "Orders",
                    new: "X"
                }
            )
            .is_err()
        );
    }
}
