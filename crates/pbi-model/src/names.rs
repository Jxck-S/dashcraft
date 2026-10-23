//! Name comparison and quoting helpers for DAX and TMDL.
//!
//! Power BI object names are case-insensitive, so every lookup in this crate
//! goes through [`names_eq`].

use std::borrow::Cow;

use crate::semantic::ColumnRef;

/// Case-insensitive name equality (Unicode lowercase folding).
pub fn names_eq(a: &str, b: &str) -> bool {
    if a.len() == b.len() && a.eq_ignore_ascii_case(b) {
        return true;
    }
    a.chars()
        .flat_map(char::to_lowercase)
        .eq(b.chars().flat_map(char::to_lowercase))
}

/// Case-folded key for hash maps keyed by object name.
pub fn name_key(name: &str) -> String {
    name.chars().flat_map(char::to_lowercase).collect()
}

/// `true` if `name` can be used as an object name: not blank and free of
/// control characters (line breaks, tabs, NUL...).
pub fn is_valid_object_name(name: &str) -> bool {
    !name.trim().is_empty() && !name.chars().any(char::is_control)
}

/// DAX keywords that must be quoted when used as a table name.
const DAX_KEYWORDS: &[&str] = &[
    "AND", "ASC", "AT", "BY", "COLUMN", "DEFINE", "DESC", "EVALUATE", "FALSE", "IN", "MEASURE",
    "NOT", "OR", "ORDER", "RETURN", "START", "TABLE", "TRUE", "VAR",
];

fn is_plain_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// DAX table reference: `Orders`, `'Web Sessions'`, `'O''Brien'`.
///
/// Names made of `[A-Za-z0-9_]` that do not start with a digit (and are not a
/// DAX keyword such as `VAR`) are left unquoted; everything else is wrapped in
/// single quotes with `'` doubled.
pub fn dax_table_ref(name: &str) -> String {
    if is_plain_identifier(name) && !DAX_KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(name)) {
        name.to_owned()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// DAX column reference: `'Web Sessions'[Session ID]` (`]` is escaped as `]]`).
pub fn dax_column_ref(table: &str, column: &str) -> String {
    format!("{}{}", dax_table_ref(table), dax_measure_ref(column))
}

/// DAX measure reference: `[Total Sales]`.
pub fn dax_measure_ref(name: &str) -> String {
    format!("[{}]", name.replace(']', "]]"))
}

/// DAX string literal: `"say ""hi"""`.
pub fn dax_string_literal(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

/// TMDL object name, quoted when it contains `.`, `=`, `:`, `'` or
/// whitespace (or is empty): `Orders`, `'Web Sessions'`.
pub fn tmdl_name(name: &str) -> Cow<'_, str> {
    let needs_quotes = name.is_empty()
        || name
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '.' | '=' | ':' | '\''));
    if needs_quotes {
        Cow::Owned(format!("'{}'", name.replace('\'', "''")))
    } else {
        Cow::Borrowed(name)
    }
}

/// Removes TMDL single-quote quoting (`'It''s'` → `It's`). Unquoted input is
/// returned trimmed.
pub fn tmdl_unquote(text: &str) -> Cow<'_, str> {
    let t = text.trim();
    if t.len() >= 2 && t.starts_with('\'') && t.ends_with('\'') {
        Cow::Owned(t[1..t.len() - 1].replace("''", "'"))
    } else {
        Cow::Borrowed(t)
    }
}

/// TMDL `table.column` reference as used by `fromColumn`/`toColumn`:
/// `Orders.'Store ID'`.
pub fn tmdl_column_ref(column: &ColumnRef) -> String {
    format!("{}.{}", tmdl_name(&column.table), tmdl_name(&column.column))
}

/// Parses a TMDL `table.column` reference (`'Web Sessions'.Device`,
/// `Orders.'Store ID'`). Returns `None` if malformed.
pub fn parse_tmdl_column_ref(text: &str) -> Option<ColumnRef> {
    let t = text.trim();
    let (table, rest) = if let Some(after) = t.strip_prefix('\'') {
        // Find the closing quote that is not part of a doubled `''`.
        let bytes = after.as_bytes();
        let mut i = 0;
        let close = loop {
            match bytes.get(i) {
                Some(b'\'') if bytes.get(i + 1) == Some(&b'\'') => i += 2,
                Some(b'\'') => break i,
                Some(_) => i += 1,
                None => return None,
            }
        };
        let table = after.get(..close)?.replace("''", "'");
        (table, after.get(close + 1..)?)
    } else {
        let dot = t.find('.')?;
        (t.get(..dot)?.trim().to_owned(), t.get(dot..)?)
    };
    let column = rest.trim_start().strip_prefix('.')?;
    let column = tmdl_unquote(column).into_owned();
    if table.is_empty() || column.is_empty() {
        return None;
    }
    Some(ColumnRef { table, column })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_compare_case_insensitively_including_unicode() {
        assert!(names_eq("Web Sessions", "web sessions"));
        assert!(names_eq("ÄPFEL", "äpfel"));
        assert!(!names_eq("Web Sessions", "Web Session"));
        assert_eq!(name_key("Store Name NAME"), "store name name");
    }

    #[test]
    fn dax_quoting_follows_identifier_rules() {
        assert_eq!(dax_table_ref("Orders"), "Orders");
        assert_eq!(dax_table_ref("Web_Sessions2"), "Web_Sessions2");
        assert_eq!(dax_table_ref("Web Sessions"), "'Web Sessions'");
        assert_eq!(dax_table_ref("2024Sales"), "'2024Sales'");
        assert_eq!(dax_table_ref("O'Brien"), "'O''Brien'");
        assert_eq!(dax_table_ref("Ventas€"), "'Ventas€'");
        assert_eq!(dax_table_ref("Var"), "'Var'");
        assert_eq!(
            dax_column_ref("Web Sessions", "Session ID"),
            "'Web Sessions'[Session ID]"
        );
        assert_eq!(dax_column_ref("T", "a]b"), "T[a]]b]");
        assert_eq!(dax_measure_ref("Margin %"), "[Margin %]");
        assert_eq!(dax_string_literal("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn tmdl_names_round_trip() {
        assert_eq!(tmdl_name("Orders"), "Orders");
        assert_eq!(tmdl_name("Web Sessions"), "'Web Sessions'");
        assert_eq!(tmdl_name("Sales (Scaled)"), "'Sales (Scaled)'");
        assert_eq!(tmdl_name("It's"), "'It''s'");
        assert_eq!(tmdl_name("a.b"), "'a.b'");
        for name in ["Orders", "Web Sessions", "It's", "a.b", ""] {
            assert_eq!(tmdl_unquote(&tmdl_name(name)), name);
        }
    }

    #[test]
    fn tmdl_column_refs_parse_and_format() {
        let r = parse_tmdl_column_ref("'Web Sessions'.Device").unwrap();
        assert_eq!(r, ColumnRef::new("Web Sessions", "Device"));
        assert_eq!(tmdl_column_ref(&r), "'Web Sessions'.Device");

        let r = parse_tmdl_column_ref("Orders.'Store ID'").unwrap();
        assert_eq!(r, ColumnRef::new("Orders", "Store ID"));

        let r = parse_tmdl_column_ref("'It''s.x'.'a.b'").unwrap();
        assert_eq!(r, ColumnRef::new("It's.x", "a.b"));
        assert_eq!(tmdl_column_ref(&r), "'It''s.x'.'a.b'");

        assert_eq!(parse_tmdl_column_ref("NoDot"), None);
        assert_eq!(parse_tmdl_column_ref("'unterminated.x"), None);
        assert_eq!(parse_tmdl_column_ref("Table."), None);
    }

    #[test]
    fn object_name_validation() {
        assert!(is_valid_object_name("Web Sessions"));
        assert!(!is_valid_object_name("   "));
        assert!(!is_valid_object_name("a\nb"));
    }
}
