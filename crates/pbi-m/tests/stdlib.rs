//! Library function tests. `check(expr, expected)` evaluates both M expressions and compares
//! them with M equality.

use std::sync::Arc;

use chrono::{FixedOffset, TimeZone, Utc};
use pbi_m::{Engine, EvalOptions, FixedClock, MError, Value};

fn engine() -> Engine {
    Engine::new(EvalOptions {
        clock: Arc::new(FixedClock {
            now: Utc
                .with_ymd_and_hms(2024, 5, 6, 7, 8, 9)
                .single()
                .unwrap_or_else(|| panic!("date")),
            offset: FixedOffset::east_opt(2 * 3600).unwrap_or_else(|| panic!("offset")),
        }),
        ..EvalOptions::default()
    })
}

fn eval(src: &str) -> Value {
    engine()
        .evaluate(src)
        .unwrap_or_else(|e| panic!("{src}\n=> {e}"))
}

#[track_caller]
fn check(src: &str, expected: &str) {
    let (a, b) = (eval(src), eval(expected));
    assert!(a == b, "{src}\n  got      {a}\n  expected {b}");
}

#[track_caller]
fn fails(src: &str, needle: &str) {
    match engine().evaluate(src) {
        Ok(v) => panic!("expected an error for {src}, got {v}"),
        Err(e) => assert!(e.to_string().contains(needle), "{src}: {e}"),
    }
}

const PEOPLE: &str = r#"#table({"Name", "Age", "City"}, {{"Ann", 30, "Oslo"}, {"Bob", null, "Rome"}, {"cid", 25, "Oslo"}, {"Ann", 30, "Oslo"}})"#;

fn with_people(body: &str) -> String {
    format!("let T = {PEOPLE} in {body}")
}

// ---------------------------------------------------------------------------------------------
// Csv.Document
// ---------------------------------------------------------------------------------------------

#[test]
fn csv_document_options() {
    check(
        r#"Csv.Document("a,b#(lf)1,2#(lf)")"#,
        r#"#table({"Column1", "Column2"}, {{"a", "b"}, {"1", "2"}})"#,
    );
    check(
        r#"Csv.Document("a;b#(cr,lf)""x;y"";z", [Delimiter = ";"])"#,
        r#"#table({"Column1", "Column2"}, {{"a", "b"}, {"x;y", "z"}})"#,
    );
    // QuoteStyle.Csv keeps line breaks inside quotes; QuoteStyle.None ends the row.
    check(
        r#"Table.RowCount(Csv.Document("""a#(lf)b"",c#(lf)d,e", [QuoteStyle = QuoteStyle.Csv]))"#,
        "2",
    );
    check(
        r#"Table.RowCount(Csv.Document("""a#(lf)b"",c#(lf)d,e", [QuoteStyle = QuoteStyle.None]))"#,
        "3",
    );
    check(
        r#"Csv.Document("a,b,c#(lf)1", [Columns = 2])"#,
        r#"#table({"Column1", "Column2"}, {{"a", "b"}, {"1", null}})"#,
    );
    check(
        r#"Table.ColumnNames(Csv.Document("1,2", [Columns = {"x", "y"}]))"#,
        r#"{"x", "y"}"#,
    );
    check(r#"Csv.Document("a|b", null, "|"){0}[Column2]"#, r#""b""#);
    check(
        r##"Csv.Document("a#(tab)b", [Delimiter = "#(tab)"]){0}[Column2]"##,
        r#""b""#,
    );
    fails(
        r#"Csv.Document("a,b,c", [Columns = 2, ExtraValues = ExtraValues.Error])"#,
        "3 fields",
    );
    check(
        r#"Csv.Document("a,b,c", [Columns = 2, ExtraValues = ExtraValues.List]){0}[Column2]"#,
        r#"{"b", "c"}"#,
    );
}

#[test]
fn csv_document_encodings() {
    // "é" in Windows-1252 and UTF-8 (with BOM), UTF-16LE with BOM.
    check(
        r#"Csv.Document(#binary({233}), [Encoding = 1252]){0}[Column1]"#,
        r#""é""#,
    );
    check(
        r#"Csv.Document(#binary({239, 187, 191, 195, 169}), [Encoding = 65001]){0}[Column1]"#,
        r#""é""#,
    );
    check(
        r#"Csv.Document(#binary({255, 254, 104, 0, 105, 0}), [Encoding = 1200]){0}[Column1]"#,
        r#""hi""#,
    );
    check(
        r#"Csv.Document(#binary({104, 105})){0}[Column1]"#,
        r#""hi""#,
    );
    fails(
        r#"Csv.Document(#binary({1}), [Encoding = 99999])"#,
        "encoding",
    );
}

#[test]
fn malformed_csv_is_an_error() {
    let e = engine()
        .evaluate(r#"Csv.Document("a,""unterminated#(lf)b,c", [QuoteStyle = QuoteStyle.Csv])"#)
        .err()
        .unwrap_or_else(|| panic!("expected an error"));
    assert!(
        matches!(&e, MError::Evaluation { reason, .. } if reason == "DataFormat.Error"),
        "{e:?}"
    );
    assert!(e.to_string().contains("never closed"), "{e}");
    fails("Csv.Document(42)", "Binary");
}

// ---------------------------------------------------------------------------------------------
// Table.*
// ---------------------------------------------------------------------------------------------

#[test]
fn table_promote_headers() {
    check(
        r#"Table.PromoteHeaders(#table(2, {{"a", "b"}, {1, 2}}))"#,
        r#"#table({"a", "b"}, {{1, 2}})"#,
    );
    check(
        r#"Table.ColumnNames(Table.PromoteHeaders(#table(3, {{"a", 5, "a"}}), [PromoteAllScalars = true]))"#,
        r#"{"a", "5", "a_1"}"#,
    );
    check(
        r#"Table.ColumnNames(Table.PromoteHeaders(#table(2, {{"a", 5}})))"#,
        r#"{"a", "Column2"}"#,
    );
}

#[test]
fn table_select_rows() {
    check(
        &with_people(r#"Table.RowCount(Table.SelectRows(T, each [City] = "Oslo"))"#),
        "3",
    );
    check(
        &with_people(r#"Table.SelectRows(T, each [Age] = null)[Name]"#),
        r#"{"Bob"}"#,
    );
    fails(&with_people("Table.SelectRows(T, each [Age])"), "Logical");
}

#[test]
fn table_rename_columns() {
    check(
        &with_people(r#"Table.ColumnNames(Table.RenameColumns(T, {"Name", "N"}))"#),
        r#"{"N", "Age", "City"}"#,
    );
    check(
        &with_people(
            r#"Table.ColumnNames(Table.RenameColumns(T, {{"Name", "Age"}, {"Age", "Name"}}))"#,
        ),
        r#"{"Age", "Name", "City"}"#,
    );
    fails(
        &with_people(r#"Table.RenameColumns(T, {"Nope", "X"})"#),
        "'Nope'",
    );
    check(
        &with_people(
            r#"Table.ColumnNames(Table.RenameColumns(T, {"Nope", "X"}, MissingField.Ignore))"#,
        ),
        r#"{"Name", "Age", "City"}"#,
    );
    check(
        &with_people(r#"Table.RenameColumns(T, {"Nope", "X"}, MissingField.UseNull)[X]"#),
        "{null, null, null, null}",
    );
    fails(
        &with_people(r#"Table.RenameColumns(T, {"Name", "Age"})"#),
        "already exists",
    );
}

#[test]
fn table_transform_columns() {
    check(
        &with_people(r#"Table.TransformColumns(T, {"Name", Text.Upper})[Name]"#),
        r#"{"ANN", "BOB", "CID", "ANN"}"#,
    );
    check(
        &with_people(
            r#"Table.TransformColumns(T, {{"Name", Text.Lower, type text}, {"Age", each _ ?? 0}})[Age]"#,
        ),
        "{30, 0, 25, 30}",
    );
    // Per-cell errors stay in their cell.
    check(
        &with_people(r#"Table.RowCount(Table.TransformColumns(T, {"Age", each error "bad"}))"#),
        "4",
    );
    fails(
        &with_people(r#"Table.TransformColumns(T, {"Age", each error "bad"}){0}[Age]"#),
        "bad",
    );
    fails(
        &with_people(r#"Table.TransformColumns(T, {"Nope", Text.Upper})"#),
        "'Nope'",
    );
    check(
        &with_people(
            r#"Table.RowCount(Table.TransformColumns(T, {"Nope", Text.Upper}, null, MissingField.Ignore))"#,
        ),
        "4",
    );
}

#[test]
fn table_transform_column_types() {
    let src = r#"let T = #table({"n", "d", "b", "i", "c", "p", "dt"}, {
        {"1,234.5", "2024-01-31", "true", "2.5", "1.23456", "50%", "2024-01-31 13:45:00"},
        {"", "1/2/2024", "FALSE", "3.5", "2", "0.25", "1/2/2024 1:00 PM"}}),
        X = Table.TransformColumnTypes(T, {{"n", type number}, {"d", type date}, {"b", type logical},
            {"i", Int64.Type}, {"c", Currency.Type}, {"p", Percentage.Type}, {"dt", type datetime}}, "en-US")
    in X"#;
    check(&format!("({src})[n]"), "{1234.5, null}");
    check(
        &format!("({src})[d]"),
        "{#date(2024, 1, 31), #date(2024, 1, 2)}",
    );
    check(&format!("({src})[b]"), "{true, false}");
    check(&format!("({src})[i]"), "{2, 4}");
    check(&format!("({src})[c]"), "{1.2346, 2}");
    check(&format!("({src})[p]"), "{0.5, 0.25}");
    check(
        &format!("({src})[dt]"),
        "{#datetime(2024, 1, 31, 13, 45, 0), #datetime(2024, 1, 2, 13, 0, 0)}",
    );
    // Conversion failures are cell errors (DataFormat.Error) raised when the cell is read.
    let bad = r#"Table.TransformColumnTypes(#table({"a"}, {{"x"}, {"1"}}), {"a", type number})"#;
    check(&format!("Table.RowCount({bad})"), "2");
    check(&format!("{bad}{{1}}[a]"), "1");
    check(
        &format!("(try {bad}{{0}}[a])[Error][Reason]"),
        r#""DataFormat.Error""#,
    );
    check(
        &format!("Value.Type({bad}) = type table [a = number]"),
        "true",
    );
}

#[test]
fn table_add_column() {
    check(
        &with_people(r#"Table.AddColumn(T, "Key", each Text.Upper([Name]), type text)[Key]"#),
        r#"{"ANN", "BOB", "CID", "ANN"}"#,
    );
    fails(
        &with_people(r#"Table.AddColumn(T, "Name", each 1)"#),
        "already exists",
    );
    check(
        &with_people(r#"Table.AddIndexColumn(T, "Idx", 1, 1)[Idx]"#),
        "{1, 2, 3, 4}",
    );
    check(
        &with_people(r#"Table.DuplicateColumn(T, "Age", "Age2")[Age2]"#),
        "{30, null, 25, 30}",
    );
}

#[test]
fn table_distinct() {
    check(&with_people("Table.RowCount(Table.Distinct(T))"), "3");
    check(
        &with_people(r#"Table.Distinct(T, {"City"})[Name]"#),
        r#"{"Ann", "Bob"}"#,
    );
    check(
        &with_people(r#"Table.Distinct(T, "Name")[Name]"#),
        r#"{"Ann", "Bob", "cid"}"#,
    );
    // Case-sensitive by default; Comparer.OrdinalIgnoreCase folds case.
    check(
        r#"Table.RowCount(Table.Distinct(#table({"a"}, {{"x"}, {"X"}})))"#,
        "2",
    );
    check(
        r#"Table.RowCount(Table.Distinct(#table({"a"}, {{"x"}, {"X"}}), {"a", Comparer.OrdinalIgnoreCase}))"#,
        "1",
    );
}

#[test]
fn table_select_and_remove_columns() {
    check(
        &with_people(r#"Table.ColumnNames(Table.SelectColumns(T, {"City", "Name"}))"#),
        r#"{"City", "Name"}"#,
    );
    fails(
        &with_people(r#"Table.SelectColumns(T, {"Nope"})"#),
        "'Nope'",
    );
    check(
        &with_people(
            r#"Table.ColumnNames(Table.SelectColumns(T, {"Name", "Nope"}, MissingField.Ignore))"#,
        ),
        r#"{"Name"}"#,
    );
    check(
        &with_people(r#"Table.SelectColumns(T, {"Nope"}, MissingField.UseNull)[Nope]"#),
        "{null, null, null, null}",
    );
    check(
        &with_people(r#"Table.ColumnNames(Table.RemoveColumns(T, "Age"))"#),
        r#"{"Name", "City"}"#,
    );
    fails(
        &with_people(r#"Table.RemoveColumns(T, {"Nope"})"#),
        "'Nope'",
    );
    check(
        &with_people(r#"Table.ColumnNames(Table.ReorderColumns(T, {"City", "Name"}))"#),
        r#"{"City", "Age", "Name"}"#,
    );
}

#[test]
fn table_shape_functions() {
    check(&with_people("Table.RowCount(T)"), "4");
    check(
        &with_people("Table.ColumnNames(T)"),
        r#"{"Name", "Age", "City"}"#,
    );
    check(
        r#"Table.FromRows({{1, "a"}, {2, "b"}}, {"n", "s"})"#,
        r#"#table({"n", "s"}, {{1, "a"}, {2, "b"}})"#,
    );
    check(
        r#"Table.FromRecords({[a = 1, b = 2], [a = 3, b = 4]})"#,
        r#"#table({"a", "b"}, {{1, 2}, {3, 4}})"#,
    );
    fails(r#"Table.FromRecords({[a = 1], [b = 2]})"#, "'a'");
    check(
        r#"Table.FromRecords({[a = 1], [b = 2]}, {"a"}, MissingField.UseNull)[a]"#,
        "{1, null}",
    );
    check(
        &with_people("Table.ToRecords(Table.FirstN(T, 1))"),
        r#"{[Name = "Ann", Age = 30, City = "Oslo"]}"#,
    );
    check(
        &with_people("Table.FirstN(T, 2)[Name]"),
        r#"{"Ann", "Bob"}"#,
    );
    check(
        &with_people("Table.FirstN(T, each [City] <> \"Rome\")[Name]"),
        r#"{"Ann"}"#,
    );
    check(&with_people("Table.Skip(T, 3)[Name]"), r#"{"Ann"}"#);
    check(
        &with_people("Table.Skip(T)[Name]"),
        r#"{"Bob", "cid", "Ann"}"#,
    );
    check(&with_people("Table.LastN(T, 1)[Name]"), r#"{"Ann"}"#);
    check(
        &with_people("Table.Range(T, 1, 2)[Name]"),
        r#"{"Bob", "cid"}"#,
    );
    check(&with_people("Table.First(T)[Name]"), r#""Ann""#);
    check(
        &with_people("Table.Column(T, \"City\")"),
        r#"{"Oslo", "Rome", "Oslo", "Oslo"}"#,
    );
    check(&with_people("Table.IsEmpty(Table.Skip(T, 10))"), "true");
    check(
        &with_people(r#"Table.HasColumns(T, {"Name", "Age"})"#),
        "true",
    );
    check(
        r#"Table.FromList({"a,b", "c,d"}, Splitter.SplitTextByDelimiter(","), {"x", "y"})"#,
        r#"#table({"x", "y"}, {{"a", "b"}, {"c", "d"}})"#,
    );
    check(
        r#"Table.TransformColumnNames(#table({"a b"}, {}), each Text.Replace(_, " ", "_"))"#,
        r#"#table({"a_b"}, {})"#,
    );
}

#[test]
fn table_sort() {
    check(
        &with_people(r#"Table.Sort(T, "Name")[Name]"#),
        r#"{"Ann", "Ann", "Bob", "cid"}"#,
    );
    check(
        &with_people(r#"Table.Sort(T, {"Age", Order.Descending})[Age]"#),
        "{30, 30, 25, null}",
    );
    check(
        &with_people(
            r#"Table.Sort(T, {{"City", Order.Ascending}, {"Name", Order.Descending}})[Name]"#,
        ),
        r#"{"cid", "Ann", "Ann", "Bob"}"#,
    );
    check(
        &with_people("Table.Sort(T, each [Age] ?? 99)[Name]"),
        r#"{"cid", "Ann", "Ann", "Bob"}"#,
    );
}

#[test]
fn table_replace_value() {
    check(
        &with_people(
            r#"Table.ReplaceValue(T, "Oslo", "OSL", Replacer.ReplaceValue, {"City"})[City]"#,
        ),
        r#"{"OSL", "Rome", "OSL", "OSL"}"#,
    );
    check(
        &with_people(r#"Table.ReplaceValue(T, "o", "0", Replacer.ReplaceText, {"City"})[City]"#),
        r#"{"Osl0", "R0me", "Osl0", "Osl0"}"#,
    );
    check(
        &with_people(r#"Table.ReplaceValue(T, null, 0, Replacer.ReplaceValue, {"Age"})[Age]"#),
        "{30, 0, 25, 30}",
    );
    check(
        &with_people(
            r#"Table.ReplaceValue(T, each [Name], each [City], Replacer.ReplaceValue, {"Name"})[Name]"#,
        ),
        r#"{"Oslo", "Rome", "Oslo", "Oslo"}"#,
    );
}

#[test]
fn table_combine() {
    check(
        r#"Table.Combine({#table({"a"}, {{1}}), #table({"a", "b"}, {{2, 3}})})"#,
        r#"#table({"a", "b"}, {{1, null}, {2, 3}})"#,
    );
    check(
        r#"#table({"a"}, {{1}}) & #table({"a"}, {{2}})"#,
        r#"#table({"a"}, {{1}, {2}})"#,
    );
}

#[test]
fn table_errors_and_fill() {
    let src =
        r#"Table.TransformColumns(#table({"a"}, {{"1"}, {"x"}, {null}}), {"a", Number.FromText})"#;
    check(
        &format!("Table.RowCount(Table.RemoveRowsWithErrors({src}))"),
        "2",
    );
    check(
        &format!("Table.RowCount(Table.SelectRowsWithErrors({src}))"),
        "1",
    );
    check(
        &format!("Table.ReplaceErrorValues({src}, {{\"a\", -1}})[a]"),
        "{1, -1, null}",
    );
    check(
        r#"Table.FillDown(#table({"a"}, {{1}, {null}, {2}, {null}}), {"a"})[a]"#,
        "{1, 1, 2, 2}",
    );
    check(
        r#"Table.FillUp(#table({"a"}, {{null}, {1}}), {"a"})[a]"#,
        "{1, 1}",
    );
}

// ---------------------------------------------------------------------------------------------
// Text.*
// ---------------------------------------------------------------------------------------------

#[test]
fn text_trim_case_length() {
    check(r#"Text.Trim("  a b#(tab) ")"#, r#""a b""#);
    check(r#"Text.Trim(null)"#, "null");
    check(r#"Text.Trim("xxaxx", "x")"#, r#""a""#);
    check(r#"Text.TrimStart("  a ")"#, r#""a ""#);
    check(r#"Text.TrimEnd("  a ")"#, r#""  a""#);
    check(r#"Text.Upper("abé")"#, r#""ABÉ""#);
    check(r#"Text.Lower("ABC")"#, r#""abc""#);
    check(
        r#"Text.Proper("the QUICK brown-fox")"#,
        r#""The Quick Brown-Fox""#,
    );
    check(r#"Text.Length("héllo")"#, "5");
    check(r#"Text.Length(null)"#, "null");
}

#[test]
fn text_substrings() {
    check(r#"Text.Start("abcdef", 2)"#, r#""ab""#);
    check(r#"Text.Start("ab", 5)"#, r#""ab""#);
    check(r#"Text.End("abcdef", 2)"#, r#""ef""#);
    check(r#"Text.Range("abcdef", 2)"#, r#""cdef""#);
    check(r#"Text.Range("abcdef", 2, 3)"#, r#""cde""#);
    check(r#"Text.Range("abc", 3)"#, r#""""#);
    fails(r#"Text.Range("abc", 4)"#, "offset");
    fails(r#"Text.Range("abc", 1, 5)"#, "count");
    check(r#"Text.Middle("abcdef", 2, 10)"#, r#""cdef""#);
    check(r#"Text.Middle("abc", 10)"#, r#""""#);
    check(r#"Text.At("abc", 1)"#, r#""b""#);
}

#[test]
fn text_delimiters() {
    check(r#"Text.AfterDelimiter("a-b-c", "-")"#, r#""b-c""#);
    check(r#"Text.AfterDelimiter("a-b-c", "-", 1)"#, r#""c""#);
    check(
        r#"Text.AfterDelimiter("a-b-c", "-", {0, RelativePosition.FromEnd})"#,
        r#""c""#,
    );
    check(r#"Text.AfterDelimiter("abc", "-")"#, r#""""#);
    check(r#"Text.BeforeDelimiter("a-b-c", "-")"#, r#""a""#);
    check(
        r#"Text.BeforeDelimiter("a-b-c", "-", {0, RelativePosition.FromEnd})"#,
        r#""a-b""#,
    );
    check(r#"Text.BeforeDelimiter("abc", "-")"#, r#""abc""#);
    check(r#"Text.BetweenDelimiters("a[b]c", "[", "]")"#, r#""b""#);
    check(r#"Text.BetweenDelimiters("abc", "[", "]")"#, r#""""#);
    check(r#"Text.Split("a, b,c", ",")"#, r#"{"a", " b", "c"}"#);
    check(r#"Text.Split("a}, {b", "}, {")"#, r#"{"a", "b"}"#);
    check(r#"Text.SplitAny("a;b,c", ",;")"#, r#"{"a", "b", "c"}"#);
}

#[test]
fn text_search_and_replace() {
    check(r#"Text.Contains("Hello", "ell")"#, "true");
    check(r#"Text.Contains("Hello", "ELL")"#, "false");
    check(
        r#"Text.Contains("Hello", "ELL", Comparer.OrdinalIgnoreCase)"#,
        "true",
    );
    check(r#"Text.StartsWith("Hello", "He")"#, "true");
    check(r#"Text.EndsWith("Hello", "lo")"#, "true");
    check(r#"Text.PositionOf("abcabc", "c")"#, "2");
    check(r#"Text.PositionOf("abcabc", "c", Occurrence.Last)"#, "5");
    check(r#"Text.PositionOf("abc", "z")"#, "-1");
    check(r#"Text.Replace("a-b-c", "-", "+")"#, r#""a+b+c""#);
    check(r#"Text.Remove("a1b2", {"1", "2"})"#, r#""ab""#);
    check(r#"Text.Select("a1b2", "12")"#, r#""12""#);
}

#[test]
fn text_combine_from_pad() {
    check(r#"Text.Combine({"a", null, "b"}, ", ")"#, r#""a, b""#);
    check(r#"Text.Combine({"a", "b"})"#, r#""ab""#);
    fails(r#"Text.Combine({"a", 1})"#, "Text");
    check("Text.From(1.5)", r#""1.5""#);
    check("Text.From(0.1 + 0.2)", r#""0.3""#);
    check("Text.From(1e20)", r#""1E+20""#);
    check("Text.From(true)", r#""true""#);
    check("Text.From(null)", "null");
    check("Text.From(#date(2024, 1, 31))", r#""1/31/2024""#);
    check(
        "Text.From(#datetime(2024, 1, 31, 13, 5, 0))",
        r#""1/31/2024 1:05:00 PM""#,
    );
    check("Text.From(#duration(1, 2, 3, 4))", r#""1.02:03:04""#);
    check(r#"Text.PadStart("7", 3, "0")"#, r#""007""#);
    check(r#"Text.PadEnd("a", 3)"#, r#""a  ""#);
    check(r#"Text.Repeat("ab", 3)"#, r#""ababab""#);
    check(r#"Text.Insert("ac", 1, "b")"#, r#""abc""#);
    check(r#"Text.Reverse("abc")"#, r#""cba""#);
    check(
        r##"Text.Format("#{0} and #{1}", {"x", 2})"##,
        r#""x and 2""#,
    );
    check(r##"Text.Format("#[a]!", [a = "hi"])"##, r#""hi!""#);
    check(r#"Text.Clean("a#(lf)b")"#, r#""ab""#);
    check(
        r#"Text.FromBinary(Text.ToBinary("é", 1252), 1252)"#,
        r#""é""#,
    );
    check("Character.ToNumber(\"A\")", "65");
    check("Character.FromNumber(97)", r#""a""#);
    check(
        r#"Lines.FromText("a#(cr,lf)b#(lf)c")"#,
        r#"{"a", "b", "c"}"#,
    );
}

// ---------------------------------------------------------------------------------------------
// List.*
// ---------------------------------------------------------------------------------------------

#[test]
fn list_functions() {
    check("List.Select({1, 2, 3, 4}, each _ > 2)", "{3, 4}");
    check("List.Transform({1, 2}, each _ * 10)", "{10, 20}");
    check("List.IsEmpty({})", "true");
    check("List.RemoveNulls({1, null, 2})", "{1, 2}");
    check("List.Count({1, null})", "2");
    check("List.NonNullCount({1, null})", "1");
    check(r#"List.Contains({"a", "b"}, "b")"#, "true");
    check(
        r#"List.Contains({"a"}, "A", Comparer.OrdinalIgnoreCase)"#,
        "true",
    );
    check("List.First({})", "null");
    check("List.First({}, 0)", "0");
    check("List.First({5, 6})", "5");
    check("List.Last({5, 6})", "6");
    check(
        r#"List.Distinct({"a", "b", "a", "A"})"#,
        r#"{"a", "b", "A"}"#,
    );
    check("List.Sum({1, null, 2.5})", "3.5");
    check("List.Sum({})", "null");
    check("List.Max({3, null, 7, 1})", "7");
    check("List.Min({3, null, 7, 1})", "1");
    check("List.Max({}, 0)", "0");
    check("List.Average({1, 2, 3, null})", "2");
    check("List.Combine({{1}, {2, 3}})", "{1, 2, 3}");
    check("List.Accumulate({1, 2, 3}, 0, (s, x) => s + x)", "6");
    check("List.Numbers(1, 3, 2)", "{1, 3, 5}");
    check("List.Reverse({1, 2})", "{2, 1}");
    check("List.Sort({3, 1, 2})", "{1, 2, 3}");
    check("List.Sort({3, 1, 2}, Order.Descending)", "{3, 2, 1}");
    check(
        "List.Sort({\"bb\", \"a\"}, each Text.Length(_))",
        r#"{"a", "bb"}"#,
    );
    check("List.FirstN({1, 2, 3}, 2)", "{1, 2}");
    check("List.Skip({1, 2, 3}, 2)", "{3}");
    check("List.LastN({1, 2, 3}, 2)", "{2, 3}");
    check("List.Range({1, 2, 3, 4}, 1, 2)", "{2, 3}");
    check("List.PositionOf({1, 2, 3}, 2)", "1");
    check("List.AnyTrue({false, true})", "true");
    check("List.AllTrue({false, true})", "false");
    check("List.MatchesAll({2, 4}, each Number.IsEven(_))", "true");
    check("List.Zip({{1, 2}, {3}})", "{{1, 3}, {2, null}}");
    check("List.Repeat({1}, 3)", "{1, 1, 1}");
    check(
        "List.Generate(() => 0, each _ < 3, each _ + 1)",
        "{0, 1, 2}",
    );
    check("List.Difference({1, 2, 2, 3}, {2})", "{1, 2, 3}");
    check("List.Union({{1, 2}, {2, 3}})", "{1, 2, 3}");
    check("List.Intersect({{1, 2}, {2, 3}})", "{2}");
    check("List.Single({7})", "7");
    fails("List.Single({1, 2})", "too many");
}

// ---------------------------------------------------------------------------------------------
// Record.*
// ---------------------------------------------------------------------------------------------

#[test]
fn record_functions() {
    check(r#"Record.Field([a = 1], "a")"#, "1");
    fails(r#"Record.Field([a = 1], "b")"#, "'b'");
    check(r#"Record.FieldOrDefault([a = 1], "b", 0)"#, "0");
    check(r#"Record.FieldOrDefault([a = 1], "a")"#, "1");
    check("Record.FieldNames([a = 1, b = 2])", r#"{"a", "b"}"#);
    check("Record.FieldValues([a = 1, b = 2])", "{1, 2}");
    check(r#"Record.HasFields([a = 1, b = 2], {"a", "b"})"#, "true");
    check(r#"Record.HasFields([a = 1], "z")"#, "false");
    check(r#"Record.AddField([a = 1], "b", 2)"#, "[a = 1, b = 2]");
    check(r#"Record.RemoveFields([a = 1, b = 2], "a")"#, "[b = 2]");
    check(r#"Record.SelectFields([a = 1, b = 2], {"b"})"#, "[b = 2]");
    check(r#"Record.RenameFields([a = 1], {"a", "x"})"#, "[x = 1]");
    check("Record.Combine({[a = 1], [b = 2]})", "[a = 1, b = 2]");
    check(
        "Record.ToTable([a = 1])",
        r#"#table({"Name", "Value"}, {{"a", 1}})"#,
    );
    check(r#"Record.FromList({1, 2}, {"a", "b"})"#, "[a = 1, b = 2]");
}

// ---------------------------------------------------------------------------------------------
// Number.*, Logical.*, Int64.From
// ---------------------------------------------------------------------------------------------

#[test]
fn number_functions() {
    check(r#"Number.From("12.5")"#, "12.5");
    check("Number.From(true)", "1");
    check(r#"Number.From("")"#, "null");
    check("Number.From(#date(1900, 1, 1))", "2");
    check(r#"Number.FromText("1,000")"#, "1000");
    let e = engine().evaluate(r#"Number.FromText("abc")"#).err();
    assert!(
        matches!(&e, Some(MError::Evaluation { reason, .. }) if reason == "DataFormat.Error"),
        "{e:?}"
    );
    check("Number.Round(2.5)", "2");
    check("Number.Round(3.5)", "4");
    check("Number.Round(1.2345, 2)", "1.23");
    check("Number.Round(2.5, 0, RoundingMode.AwayFromZero)", "3");
    check("Number.RoundUp(1.1)", "2");
    check("Number.RoundDown(-1.1)", "-2");
    check(r#"Number.ToText(1234.5, "N1")"#, r#""1,234.5""#);
    check("Number.ToText(42)", r#""42""#);
    check("Number.Abs(-3)", "3");
    check("Number.Mod(7, 3)", "1");
    check("Number.IntegerDivide(7, 2)", "3");
    check("Number.Power(2, 10)", "1024");
    check("Int64.From(\"2.5\")", "2");
    check("Int64.From(2.5, null, RoundingMode.AwayFromZero)", "3");
    check("Currency.From(1.234567)", "1.2346");
    check(r#"Logical.From("TRUE")"#, "true");
    check("Logical.From(0)", "false");
    check(r#"Logical.FromText("false")"#, "false");
    fails(r#"Logical.FromText("yes")"#, "Logical");
    check("Logical.ToText(true)", r#""true""#);
}

// ---------------------------------------------------------------------------------------------
// Date/time
// ---------------------------------------------------------------------------------------------

#[test]
fn datetime_functions() {
    check(
        "DateTimeZone.UtcNow()",
        "#datetimezone(2024, 5, 6, 7, 8, 9, 0, 0)",
    );
    check(
        "DateTimeZone.RemoveZone(DateTimeZone.UtcNow())",
        "#datetime(2024, 5, 6, 7, 8, 9)",
    );
    check("DateTime.LocalNow()", "#datetime(2024, 5, 6, 9, 8, 9)");
    check(
        "DateTimeZone.SwitchZone(#datetimezone(2024, 1, 1, 12, 0, 0, 0, 0), 3)",
        "#datetimezone(2024, 1, 1, 15, 0, 0, 3, 0)",
    );
    check(r#"Date.From("2024-01-31")"#, "#date(2024, 1, 31)");
    check(
        "Date.From(#datetime(2024, 1, 31, 10, 0, 0))",
        "#date(2024, 1, 31)",
    );
    check("Date.From(45000)", "#date(2023, 3, 15)");
    check(
        r#"DateTime.From("2024-01-31T10:20:30")"#,
        "#datetime(2024, 1, 31, 10, 20, 30)",
    );
    check(
        "DateTime.From(#date(2024, 1, 31))",
        "#datetime(2024, 1, 31, 0, 0, 0)",
    );
    check(
        "DateTime.From(#datetimezone(2024, 1, 31, 10, 0, 0, 0, 0))",
        "#datetime(2024, 1, 31, 12, 0, 0)",
    );
    check(
        r#"DateTimeZone.From("2024-01-31T10:00:00Z")"#,
        "#datetimezone(2024, 1, 31, 10, 0, 0, 0, 0)",
    );
    check(r#"Time.From("13:45")"#, "#time(13, 45, 0)");
    check(r#"Duration.From("1.00:00:00")"#, "#duration(1, 0, 0, 0)");
    check(
        "Date.AddMonths(#date(2024, 1, 31), 1)",
        "#date(2024, 2, 29)",
    );
    check(
        "Date.AddDays(#date(2024, 1, 31), -31)",
        "#date(2023, 12, 31)",
    );
    check("Date.EndOfMonth(#date(2024, 2, 10))", "#date(2024, 2, 29)");
    check("Date.DayOfWeek(#date(2024, 5, 6))", "1");
    check(
        r#"Date.ToText(#date(2024, 5, 6), "yyyy-MM-dd")"#,
        r#""2024-05-06""#,
    );
    check(
        r#"DateTime.ToText(#datetime(2024, 5, 6, 7, 8, 9), [Format = "dd/MM HH:mm"])"#,
        r#""06/05 07:08""#,
    );
}

// ---------------------------------------------------------------------------------------------
// Value.*, types, constants
// ---------------------------------------------------------------------------------------------

#[test]
fn value_and_type_functions() {
    check("Value.Is(1, type number)", "true");
    check("Value.Is(null, type nullable text)", "true");
    check("Value.Is(\"a\", Int64.Type)", "false");
    check("Value.Type(1) = type number", "true");
    check("Value.Type({}) = type {any}", "true");
    check("Value.Equals({1}, {1})", "true");
    check("Value.Compare(1, 2)", "-1");
    check(
        r#"Table.ColumnNames(Value.ReplaceType(#table({"a"}, {}), type table [b = text]))"#,
        r#"{"b"}"#,
    );
    check("Type.Is(type number, type nullable number)", "true");
    check("Type.Is(type text, type number)", "false");
    check("Type.NonNullable(type nullable text) = type text", "true");
    check("QuoteStyle.Csv", "1");
    check("MissingField.UseNull", "2");
    check("ExtraValues.Ignore", "2");
    check("Int64.Type is type", "true");
    check(
        "Value.Is(1, Percentage.Type) and Value.Is(1, Currency.Type)",
        "true",
    );
    check(r#"Function.Invoke(Text.Upper, {"a"})"#, r#""A""#);
    check(
        r#"Json.Document("{""a"": [1, true, null]}")"#,
        "[a = {1, true, null}]",
    );
    check(
        r#"Diagnostics.Trace(TraceLevel.Information, "msg", 5)"#,
        "5",
    );
}

#[test]
fn library_lists_spec_functions() {
    let names = pbi_m::library_names();
    for required in [
        "Csv.Document",
        "Table.PromoteHeaders",
        "Table.SelectRows",
        "Table.RenameColumns",
        "Table.TransformColumns",
        "Table.TransformColumnTypes",
        "Table.AddColumn",
        "Table.Distinct",
        "Table.SelectColumns",
        "Table.RemoveColumns",
        "Table.ReplaceValue",
        "Table.Combine",
        "AzureStorage.Blobs",
        "DateTimeZone.UtcNow",
        "Value.Is",
        "Value.Type",
        "Int64.Type",
        "Currency.Type",
        "Percentage.Type",
        "Replacer.ReplaceText",
    ] {
        assert!(names.contains(&required), "{required} missing");
    }
}
