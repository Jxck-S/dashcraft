#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The DAX language service (M7.1): diagnostics with spans and "did you
//! mean" suggestions, on a synthetic model.

use pbi_dax::service::{
    Diagnostic, DiagnosticCode, MAX_DIAGNOSTICS, Schema, SourceKind, diagnostics,
};
use pbi_model::{Column, DataType, Measure, Model, PropValue, RawObject, Severity, Table};

fn model() -> Model {
    let mut sales = Table::new("Sales");
    sales.columns = vec![
        Column::data("Amount", DataType::Double),
        Column::data("Order Date", DataType::DateTime),
        Column::calculated("Margin", DataType::Double, "Sales[Amount] * 0.1"),
    ];
    sales.measures = vec![Measure::new("Total Sales", "SUM(Sales[Amount])")];
    let mut category = Table::new("Product Category");
    category.columns = vec![Column::data("Category", DataType::String)];
    let mut model = Model {
        tables: vec![sales, category],
        ..Model::default()
    };
    model.extra.insert(
        "function".into(),
        PropValue::Object(Box::new(RawObject {
            name: Some("Twice".into()),
            expression: Some("(x) => x * 2".into()),
            ..RawObject::default()
        })),
    );
    model
}

fn check(src: &str) -> Vec<Diagnostic> {
    diagnostics(src, SourceKind::detect(src), &Schema::from_model(&model()))
}

/// `(code, text the span covers, suggestion)` of each diagnostic.
fn summary(src: &str) -> Vec<(&'static str, &str, Option<String>)> {
    check(src)
        .into_iter()
        .map(|d| (d.code.as_str(), d.span.text(src), d.suggestion))
        .collect()
}

fn one(src: &str) -> Diagnostic {
    let mut all = check(src);
    assert_eq!(all.len(), 1, "{src}: {all:?}");
    all.remove(0)
}

#[test]
fn valid_expressions_have_no_diagnostics() {
    for src in [
        "",
        "  \n ",
        "SUM(Sales[Amount])",
        "sum(sales[amount]) + [total sales]",
        "[Total Sales] * 2",
        "Sales[Total Sales]",
        "[Margin]",
        "CALCULATE([Total Sales], 'Product Category'[Category] = \"Bikes\")",
        "VAR _t = FILTER(Sales, Sales[Amount] > 0) RETURN SUMX(_t, [Amount])",
        "VAR _t = FILTER(Sales, Sales[Amount] > 0) RETURN SUMX(_t, _t[Amount])",
        "VAR x = 1 RETURN x + 1",
        "SUMX(ADDCOLUMNS(Sales, \"Double\", Sales[Amount] * 2), [Double])",
        "SUMX({1, 2, 3}, [Value])",
        "MAXX({(1, 2)}, [Value2])",
        "MAXX(GENERATESERIES(1, 10), [Value])",
        "MAXX(CALENDAR(DATE(2024, 1, 1), DATE(2024, 12, 31)), [Date])",
        "Twice([Total Sales])",
        "twice(1)",
        "NAMEOF(Sales[Amount], TABLE) & NAMEOF([Total Sales], (self))",
    ] {
        assert!(check(src).is_empty(), "{src:?}: {:?}", check(src));
    }
}

#[test]
fn syntax_errors_cover_the_offending_text() {
    let d = one("SUM(Sales[Amount]");
    assert_eq!(d.code, DiagnosticCode::Syntax);
    assert_eq!(d.severity, Severity::Error);
    // At the end of the text the span covers the last character.
    assert_eq!(d.span.text("SUM(Sales[Amount]"), "]");

    let src = "1 +\n  * 2";
    let d = one(src);
    assert_eq!(d.code, DiagnosticCode::Syntax);
    assert!(!d.span.is_empty());
    assert_eq!(d.range(src).0, (2, 3));
}

#[test]
fn unknown_tables_suggest_the_closest_one() {
    let d = one("COUNTROWS(Sale)");
    assert_eq!(d.code, DiagnosticCode::UnknownTable);
    // Unquoted, it may be a misspelt variable.
    assert_eq!(d.message, "unknown table or variable `Sale`");
    assert_eq!(d.suggestion.as_deref(), Some("did you mean table `Sales`?"));

    // `NAMEOF`'s component keyword is not a table; its reference is.
    assert_eq!(
        one("NAMEOF(Sale, TABLE)").span.text("NAMEOF(Sale, TABLE)"),
        "Sale"
    );

    let d = one("COUNTROWS('Product Categry')");
    assert_eq!(d.message, "unknown table `'Product Categry'`");
    assert_eq!(
        d.suggestion.as_deref(),
        Some("did you mean table `'Product Category'`?")
    );

    // The table part of a column reference.
    assert_eq!(
        summary("SUM(Salez[Amount]) + SUM('Sales Tbl'[Amount])"),
        [
            (
                "unknownTable",
                "Salez",
                Some("did you mean table `Sales`?".into())
            ),
            (
                "unknownTable",
                "'Sales Tbl'",
                Some("did you mean table `Sales`?".into())
            ),
        ]
    );
}

#[test]
fn unknown_columns_and_measures_suggest_the_closest_name() {
    assert_eq!(
        summary("SUM(Sales[Amont]) + 'Product Category'[Categories]"),
        [
            (
                "unknownColumn",
                "[Amont]",
                Some("did you mean `Sales[Amount]`?".into())
            ),
            (
                "unknownColumn",
                "[Categories]",
                Some("did you mean `'Product Category'[Category]`?".into())
            ),
        ]
    );
    let d = one("[Total Sale] / 2");
    assert_eq!(d.code, DiagnosticCode::UnknownMeasure);
    assert_eq!(d.message, "unknown measure or column `[Total Sale]`");
    assert_eq!(
        d.suggestion.as_deref(),
        Some("did you mean `[Total Sales]`?")
    );
    // A name added by another expression is no excuse on another table.
    assert_eq!(
        summary("SUMX(ADDCOLUMNS(Sales, \"Double\", 1), [Double]) + [Tripled]")
            .into_iter()
            .map(|(code, text, _)| (code, text))
            .collect::<Vec<_>>(),
        [("unknownMeasure", "[Tripled]")]
    );
}

#[test]
fn unsupported_functions_are_warnings_on_the_name() {
    let src = "summ(Sales[Amount]) + FOOBAR(1)";
    let all = check(src);
    assert_eq!(all.len(), 2, "{all:?}");
    assert!(
        all.iter()
            .all(|d| d.code == DiagnosticCode::UnsupportedFunction
                && d.severity == Severity::Warning)
    );
    let first = all.first().unwrap();
    assert_eq!(first.span.text(src), "summ");
    assert_eq!(first.suggestion.as_deref(), Some("did you mean `SUM`?"));
    assert_eq!(all.get(1).unwrap().span.text(src), "FOOBAR");
}

#[test]
fn queries_see_their_define_entries() {
    let src = "DEFINE
    VAR _min = 10
    MEASURE Sales[Big Sales] = CALCULATE([Total Sales], Sales[Amount] > _min)
    COLUMN Sales[Doubled] = Sales[Amount] * 2
    TABLE Top = TOPN(5, Sales, [Total Sales])
    FUNCTION Thrice = (x) => x * 3
EVALUATE
    SUMMARIZECOLUMNS(
        'Product Category'[Category],
        \"Big\", [Big Sales],
        \"Doubled\", SUMX(Sales, Sales[Doubled]),
        \"Top\", COUNTROWS(Top),
        \"Thrice\", Thrice(Twice(1))
    )
EVALUATE Top
EVALUATE SELECTCOLUMNS(Top, \"A\", Top[Anything])";
    assert_eq!(SourceKind::detect(src), SourceKind::Query);
    assert!(check(src).is_empty(), "{:?}", check(src));

    let src = "DEFINE MEASURE Salez[M] = 1\nEVALUATE {[M]}";
    assert_eq!(
        summary(src),
        [(
            "unknownTable",
            "MEASURE Salez[M]",
            Some("did you mean table `Sales`?".into())
        )]
    );
    // Without DEFINE entries, a query is checked against the model only.
    assert_eq!(
        summary("EVALUATE ROW(\"x\", [Nope])")
            .into_iter()
            .map(|(code, text, _)| (code, text))
            .collect::<Vec<_>>(),
        [("unknownMeasure", "[Nope]")]
    );
}

#[test]
fn source_kind_is_detected_from_the_first_token() {
    assert_eq!(
        SourceKind::detect("  -- note\n evaluate Sales"),
        SourceKind::Query
    );
    assert_eq!(
        SourceKind::detect("DEFINE VAR x = 1 EVALUATE {x}"),
        SourceKind::Query
    );
    assert_eq!(SourceKind::detect("[Total Sales]"), SourceKind::Expression);
    assert_eq!(SourceKind::detect(""), SourceKind::Expression);
}

#[test]
fn spans_and_ranges_respect_multibyte_text() {
    let src = "\"Ünïcödé\" & [Tötäl Sälés]\n+ 'Prödüct'[X]";
    let all = check(src);
    let texts: Vec<_> = all.iter().map(|d| d.span.text(src)).collect();
    assert_eq!(texts, ["[Tötäl Sälés]", "'Prödüct'"]);
    let first = all.first().unwrap();
    // Columns count characters, not bytes.
    assert_eq!(first.range(src), ((1, 13), (1, 26)));
    assert_eq!(all.get(1).unwrap().range(src).0, (2, 3));
}

#[test]
fn the_number_of_diagnostics_is_capped() {
    let args = vec!["[Nope]"; MAX_DIAGNOSTICS + 500].join(", ");
    let src = format!("COALESCE({args})");
    let all = check(&src);
    assert_eq!(all.len(), MAX_DIAGNOSTICS);
    assert!(all.windows(2).all(|w| w[0].span.start < w[1].span.start));
}

#[test]
fn without_a_model_only_syntax_and_functions_are_checked() {
    let src = "SUM(Sales[Amount]) + [M] + NOPE(1)";
    let codes = |schema: &Schema| -> Vec<_> {
        diagnostics(src, SourceKind::Expression, schema)
            .into_iter()
            .map(|d| d.code)
            .collect()
    };
    // A model without tables: every reference is unknown.
    assert_eq!(
        codes(&Schema::new()),
        [
            DiagnosticCode::UnknownTable,
            DiagnosticCode::UnknownMeasure,
            DiagnosticCode::UnsupportedFunction
        ]
    );
    assert_eq!(
        codes(&Schema::unknown()),
        [DiagnosticCode::UnsupportedFunction]
    );
    let query = "DEFINE MEASURE T[M] = 1 EVALUATE {[M]}";
    assert!(diagnostics(query, SourceKind::Query, &Schema::unknown()).is_empty());
    assert_eq!(
        diagnostics("1 +", SourceKind::Expression, &Schema::unknown())
            .first()
            .map(|d| d.code),
        Some(DiagnosticCode::Syntax)
    );
}
