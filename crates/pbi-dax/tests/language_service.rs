#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The DAX language service (M7.1): diagnostics with spans and "did you
//! mean" suggestions, signature help and hover, on a synthetic model.

use pbi_dax::service::{
    CompletionKind, Completions, Diagnostic, DiagnosticCode, Hover, HoverKind, MAX_DIAGNOSTICS,
    Schema, SourceKind, complete, diagnostics, hover, signature_help,
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

#[test]
fn wrong_argument_counts_are_errors_with_the_signature() {
    let src = "SUM()";
    let d = one(src);
    assert_eq!(
        (d.code, d.severity, d.span.text(src)),
        (DiagnosticCode::ArgumentCount, Severity::Error, "SUM")
    );
    assert_eq!(
        d.message,
        "`SUM` takes 1 argument but 0 were given: SUM(column)"
    );

    // Too many: the extra arguments are underlined.
    let src = "if(TRUE, 1, 2, 3, 4)";
    let d = one(src);
    assert_eq!(d.span.text(src), "3, 4");
    assert_eq!(
        d.message,
        "`if` takes 2 to 3 arguments but 5 were given: IF(logicalTest, valueIfTrue, [valueIfFalse])"
    );
    assert!(
        one("SWITCH(1)")
            .message
            .contains("at least 3 arguments but 1 was given")
    );
    assert_eq!(
        summary("SUMX(Sales, ROUND(Sales[Amount]))"),
        [("argumentCount", "ROUND", None)]
    );
    // Repeating parameters have no maximum.
    for src in [
        "SWITCH(1, 1, \"a\", 2, \"b\", 3, \"c\", \"z\")",
        "COALESCE(1, 2, 3, 4, 5)",
        "CALCULATE([Total Sales], Sales[Amount] > 1, Sales[Amount] < 9, ALL(Sales))",
        "TODAY()",
        "COUNTROWS()",
    ] {
        assert!(check(src).is_empty(), "{src:?}: {:?}", check(src));
    }
}

/// `marked` without its `¦` cursor, and the cursor's byte offset.
fn cursor(marked: &str) -> (String, usize) {
    let offset = marked.find('¦').expect("a ¦ cursor");
    (marked.replacen('¦', "", 1), offset)
}

/// `(function, argument, active parameter)` at the cursor.
fn sig(marked: &str) -> Option<(&'static str, usize, Option<usize>)> {
    let (src, offset) = cursor(marked);
    signature_help(&src, offset).map(|h| (h.function.name, h.argument, h.active_parameter))
}

#[test]
fn signature_help_follows_the_argument_under_the_cursor() {
    assert_eq!(sig("SUM(¦"), Some(("SUM", 0, Some(0))));
    assert_eq!(sig("IF(a > 1, ¦"), Some(("IF", 1, Some(1))));
    assert_eq!(sig("IF(a > 1, 2, 3¦)"), Some(("IF", 2, Some(2))));
    assert_eq!(sig("if(a, b, c, ¦"), Some(("IF", 3, None)));
    // The innermost call; groups, row constructors and lists are skipped.
    assert_eq!(
        sig("CALCULATE(SUM(Sales[Amount]), ¦"),
        Some(("CALCULATE", 1, Some(1)))
    );
    assert_eq!(
        sig("CALCULATE(SUM(¦Sales[Amount]), x"),
        Some(("SUM", 0, Some(0)))
    );
    assert_eq!(sig("ROUND((1 + 2) * 3, ¦"), Some(("ROUND", 1, Some(1))));
    assert_eq!(sig("ROUND((1, ¦"), Some(("ROUND", 0, Some(0))));
    assert_eq!(sig("SUMX({1, 2, 3}, ¦"), Some(("SUMX", 1, Some(1))));
    assert_eq!(
        sig("CONCATENATE(\"a, (b\", ¦"),
        Some(("CONCATENATE", 1, Some(1)))
    );
    assert_eq!(
        sig("CALCULATE(VAR a = 1 RETURN (a¦"),
        Some(("CALCULATE", 0, Some(0)))
    );
    assert_eq!(sig("CALCULATE([M], NOT(¦"), Some(("NOT", 0, Some(0))));
    // A repeating pair cycles: value, result, value, result, …
    assert_eq!(
        sig("SWITCH(x, 1, \"a\", 2, ¦"),
        Some(("SWITCH", 4, Some(4)))
    );
    assert_eq!(
        sig("SWITCH(x, 1, \"a\", 2, \"b\", ¦"),
        Some(("SWITCH", 5, Some(3)))
    );
    let (src, offset) = cursor("ROUND(Sales[Amount], ¦)");
    let help = signature_help(&src, offset).unwrap();
    assert_eq!(help.name_span.text(&src), "ROUND");
    assert_eq!(help.function.signature, "ROUND(number, numDigits)");
    for marked in [
        "¦SUM(1)",
        "SUM¦(1)",
        "SUM(1)¦",
        "SUM(1) + ¦",
        "SUM(x -- a note, ¦",
        "SUM(x /* a, ¦ */",
        "Twice(¦",
        "RETURN (¦",
        "¦",
    ] {
        assert_eq!(sig(marked), None, "{marked}");
    }
    // Offsets past the end or inside a character are clamped.
    assert_eq!(signature_help("SUM(", 99).map(|h| h.argument), Some(0));
    assert_eq!(signature_help("SUM(\"é\"", 6).map(|h| h.argument), Some(0));
}

fn hover_at(marked: &str) -> Hover {
    let (src, offset) = cursor(marked);
    hover(&src, offset, &Schema::from_model(&model()))
        .unwrap_or_else(|| panic!("{marked}: no hover"))
}

#[test]
fn hover_describes_functions_and_keywords() {
    let h = hover_at("SU¦M(Sales[Amount])");
    assert_eq!((h.kind, h.name.as_str()), (HoverKind::Function, "SUM"));
    assert_eq!(h.signature.as_deref(), Some("SUM(column)"));
    assert_eq!(h.returns, Some("number"));
    assert!(h.supported);
    assert_eq!(
        h.markdown(),
        "```dax\nSUM(column)\n```\nReturns number.\n\nAdds up the numbers in a column."
    );
    // The token that ends at the cursor counts too.
    assert_eq!(hover_at("sum¦(1)").name, "SUM");

    let h = hover_at("twi¦ce(1)");
    assert_eq!(
        (h.kind, h.name.as_str()),
        (HoverKind::UserFunction, "Twice")
    );

    let h = hover_at("V¦AR x = 1 RETURN x");
    assert_eq!((h.kind, h.name.as_str()), (HoverKind::Keyword, "VAR"));
    assert!(h.description.is_some());
    let h = hover_at("DATEADD(Sales[Order Date], -1, mon¦th)");
    assert_eq!((h.kind, h.name.as_str()), (HoverKind::Keyword, "MONTH"));

    // Functions work without a model.
    let h = hover("ROUND(1, 2)", 1, &Schema::unknown()).unwrap();
    assert_eq!(h.kind, HoverKind::Function);
}

#[test]
fn hover_shows_model_details_of_tables_columns_and_measures() {
    let h = hover_at("SUM(Sa¦les[Amount])");
    assert_eq!((h.kind, h.name.as_str()), (HoverKind::Table, "Sales"));
    assert_eq!(h.counts, Some((3, 1)));
    assert_eq!(h.markdown(), "`Sales`: table, 3 columns, 1 measure");

    let h = hover_at("SUM(Sales[Amou¦nt])");
    assert_eq!(
        (h.kind, h.name.as_str()),
        (HoverKind::Column, "Sales[Amount]")
    );
    assert_eq!(h.table.as_deref(), Some("Sales"));
    assert_eq!(h.details.data_type.as_deref(), Some("double"));
    assert_eq!(h.markdown(), "`Sales[Amount]`: column (double)");

    let h = hover_at("[Mar¦gin] + 1");
    assert_eq!(h.kind, HoverKind::Column);
    assert_eq!(h.details.expression.as_deref(), Some("Sales[Amount] * 0.1"));

    let h = hover_at("[total sales¦]");
    assert_eq!(
        (h.kind, h.name.as_str()),
        (HoverKind::Measure, "[Total Sales]")
    );
    assert_eq!(
        h.markdown(),
        "`[Total Sales]`: measure in `Sales`\n\n```dax\nSUM(Sales[Amount])\n```"
    );

    let h = hover_at("COUNTROWS('Product Cat¦egory')");
    assert_eq!(
        (h.kind, h.name.as_str()),
        (HoverKind::Table, "'Product Category'")
    );
    let h = hover_at("'Product Category'[Cate¦gory]");
    assert_eq!(h.name, "'Product Category'[Category]");
}

#[test]
fn hover_shows_variables_and_query_definitions() {
    let h = hover_at("VAR total = SUM(Sales[Amount]) VAR x = 2 RETURN tot¦al * x");
    assert_eq!((h.kind, h.name.as_str()), (HoverKind::Variable, "total"));
    assert_eq!(h.details.expression.as_deref(), Some("SUM(Sales[Amount])"));
    let h = hover_at("VAR to¦tal = 1 RETURN total");
    assert_eq!(h.details.expression.as_deref(), Some("1"));
    let h = hover_at("SUMX(Sales, VAR a = Sales[Amount] RETURN a¦ * 2)");
    assert_eq!(h.details.expression.as_deref(), Some("Sales[Amount]"));

    let q = "DEFINE MEASURE Sales[Double] = [Total Sales] * 2 VAR _n = 3 \
             EVALUATE ROW(\"x\", [Dou¦ble] * _n)";
    let h = hover_at(q);
    assert_eq!((h.kind, h.name.as_str()), (HoverKind::Measure, "[Double]"));
    assert_eq!(h.details.expression.as_deref(), Some("[Total Sales] * 2"));
    let h = hover_at(&q.replace("¦", "").replace("* _n)", "* _¦n)"));
    assert_eq!(h.details.expression.as_deref(), Some("3"));
    // A query measure replaces the model's.
    let h = hover_at("DEFINE MEASURE Sales[Total Sales] = 1 EVALUATE ROW(\"x\", [Total ¦Sales])");
    assert_eq!(h.details.expression.as_deref(), Some("1"));
    let h = hover_at(
        "DEFINE COLUMN Sales[Half] = Sales[Amount] / 2 \
         EVALUATE SELECTCOLUMNS(Sales, \"h\", Sales[Ha¦lf])",
    );
    assert_eq!(h.kind, HoverKind::Column);
    assert_eq!(h.details.expression.as_deref(), Some("Sales[Amount] / 2"));
    let h = hover_at("DEFINE TABLE Top = TOPN(1, Sales) EVALUATE To¦p");
    assert_eq!((h.kind, h.counts), (HoverKind::Table, Some((0, 0))));
}

#[test]
fn hover_is_none_for_unknown_names_literals_and_comments() {
    let schema = Schema::from_model(&model());
    for marked in [
        "SUM(No¦pe[x])",
        "\"te¦xt\"",
        "12¦3",
        "-- SU¦M(1)",
        "[Unkn¦own]",
        "Sales[Unkn¦own]",
        "Myst¦ery(1)",
        "1 +¦ 2",
        "¦",
    ] {
        let (src, offset) = cursor(marked);
        assert_eq!(hover(&src, offset, &schema), None, "{marked}");
    }
    assert_eq!(hover("'Ünïcode'", 2, &schema), None);
}

fn comp(marked: &str) -> Completions {
    let (src, offset) = cursor(marked);
    complete(&src, offset, &Schema::from_model(&model()), 0)
}

fn labels(marked: &str) -> Vec<String> {
    comp(marked).items.into_iter().map(|i| i.label).collect()
}

fn first(marked: &str) -> String {
    labels(marked).into_iter().next().unwrap_or_default()
}

#[test]
fn completion_matches_functions_names_and_keywords() {
    let c = comp("SU¦");
    assert_eq!((c.span.start, c.span.end, c.prefix.as_str()), (0, 2, "SU"));
    let sum = c.items.iter().find(|i| i.label == "SUM").unwrap();
    assert_eq!(sum.kind, CompletionKind::Function);
    assert_eq!(sum.insert_text, "SUM(");
    assert_eq!(sum.detail, "SUM(column) → number");
    assert!(sum.description.is_some());
    assert!(
        c.items
            .iter()
            .all(|i| i.label.to_lowercase().contains("su"))
    );
    // An exact name first; any case.
    assert_eq!(first("sum¦"), "SUM");
    assert_eq!(first("var¦"), "VAR");
    // A function without arguments gets its parentheses; a `(` that is
    // already there is not doubled, and the whole name is replaced.
    let pi = comp("PI¦")
        .items
        .into_iter()
        .find(|i| i.label == "PI")
        .unwrap();
    assert_eq!(pi.insert_text, "PI()");
    let c = comp("SU¦M(Sales[Amount])");
    assert_eq!((c.span.start, c.span.end), (0, 3));
    let sum = c.items.iter().find(|i| i.label == "SUM").unwrap();
    assert_eq!(sum.insert_text, "SUM");
    // Tables, then their columns (qualified), then measures (`Sal` starts a
    // word of `Total Sales`).
    assert_eq!(
        labels("Sal¦"),
        [
            "Sales",
            "Sales[Amount]",
            "Sales[Margin]",
            "Sales[Order Date]",
            "[Total Sales]"
        ]
    );
    let c = comp("Tot¦");
    let m = c.items.iter().find(|i| i.label == "[Total Sales]").unwrap();
    assert_eq!(
        (m.kind, m.table.as_deref()),
        (CompletionKind::Measure, Some("Sales"))
    );
    assert_eq!(m.insert_text, "[Total Sales]");
    let twice = comp("Twi¦").items.into_iter().next().unwrap();
    assert_eq!(
        (twice.kind, twice.insert_text.as_str()),
        (CompletionKind::UserFunction, "Twice(")
    );
    // A word of the name: `Cat` finds `'Product Category'` after the
    // column whose name starts with it, and names that contain it last.
    assert_eq!(
        labels("Cat¦"),
        [
            "'Product Category'[Category]",
            "'Product Category'",
            "CONCATENATE",
            "CONCATENATEX"
        ]
    );
    let col = comp("Amo¦").items.into_iter().next().unwrap();
    assert_eq!(
        (col.label.as_str(), col.detail.as_str()),
        ("Sales[Amount]", "column (double)")
    );
    // A qualifier: only tables.
    assert_eq!(labels("SUM(Sal¦es[Amount])"), ["Sales"]);
}

#[test]
fn completion_inside_brackets_and_quotes() {
    let c = comp("[Tot¦");
    assert_eq!((c.span.start, c.span.end, c.prefix.as_str()), (0, 4, "Tot"));
    assert_eq!(labels("[Tot¦"), ["[Total Sales]"]);
    // Measures first, then every column once.
    assert_eq!(
        labels("[¦]"),
        [
            "[Total Sales]",
            "[Amount]",
            "[Category]",
            "[Margin]",
            "[Order Date]"
        ]
    );
    assert_eq!((comp("[¦]").span.start, comp("[¦]").span.end), (0, 2));
    // Qualified: that table's columns and measures.
    assert_eq!(
        labels("Sales[¦"),
        ["[Total Sales]", "[Amount]", "[Margin]", "[Order Date]"]
    );
    let c = comp("'Product Category'[C¦]");
    assert_eq!((c.span.start, c.span.end), (18, 21));
    assert_eq!(labels("'Product Category'[C¦]"), ["[Category]"]);
    assert_eq!(
        labels("'Prod¦"),
        ["'Product Category'", "'Product Category'[Category]"]
    );
    // Right after a quoted table name.
    let c = comp("'Product Category'¦");
    assert_eq!((c.span.start, c.span.end), (18, 18));
    assert_eq!(labels("'Product Category'¦"), ["[Category]"]);
    assert!(labels("Nope[¦").is_empty());
}

#[test]
fn completion_offers_the_variables_in_scope() {
    let c = comp("VAR total = 1 VAR half = total / 2 RETURN t¦");
    let v = c.items.first().unwrap();
    assert_eq!(
        (v.label.as_str(), v.kind),
        ("total", CompletionKind::Variable)
    );
    // Not inside its own definition.
    assert!(!labels("VAR total = to¦").contains(&"total".to_string()));
    // Out of scope after the argument that declared it.
    let vars = |marked: &str| -> Vec<String> {
        comp(marked)
            .items
            .into_iter()
            .filter(|i| i.kind == CompletionKind::Variable)
            .map(|i| i.label)
            .collect()
    };
    assert!(vars("SUMX(Sales, VAR x = 1 RETURN x) + ¦").is_empty());
    assert_eq!(
        vars("VAR a = 1 RETURN SUMX(Sales, VAR b = 2 RETURN a + ¦"),
        ["a", "b"]
    );
    // Declaring a name: nothing to suggest.
    assert!(labels("VAR na¦").is_empty());
    assert!(labels("VAR x ¦").is_empty());
}

#[test]
fn completion_after_operands_and_in_queries() {
    assert_eq!(labels("[Total Sales] ¦"), ["IN", "RETURN", "VAR"]);
    assert_eq!(labels("[Total Sales] i¦"), ["IN"]);
    let (src, _) = cursor("¦");
    let c = complete(&src, 0, &Schema::from_model(&model()), 10);
    assert_eq!(c.items.len(), 10);
    assert!(c.incomplete);
    let all = labels("¦");
    assert!(all.contains(&"EVALUATE".to_string()) && all.contains(&"DEFINE".to_string()));
    assert!(!labels("SUM(¦").contains(&"EVALUATE".to_string()));
    assert_eq!(labels("EVALUATE Sales ORDER ¦"), ["BY"]);
    assert!(labels("EVALUATE Sales ¦").contains(&"ORDER BY".to_string()));
    assert_eq!(
        labels("DEFINE ¦"),
        ["COLUMN", "FUNCTION", "MEASURE", "TABLE", "VAR"]
    );
    assert_eq!(labels("DEFINE MEASURE ¦"), ["'Product Category'", "Sales"]);
    // DEFINE entries count while the query does not parse yet.
    assert_eq!(
        labels("DEFINE MEASURE Sales[Profit] = [Total Sales] * 0.2 EVALUATE ROW(\"p\", [Pro¦"),
        ["[Profit]"]
    );
    let c = comp("DEFINE TABLE Top = TOPN(1, Sales) EVALUATE To¦");
    assert!(
        c.items
            .iter()
            .any(|i| i.label == "Top" && i.kind == CompletionKind::Table)
    );
}

#[test]
fn completion_ranks_what_the_active_parameter_takes_first() {
    let kind = |marked: &str| comp(marked).items.first().map(|i| i.kind);
    assert_eq!(kind("SUMX(¦"), Some(CompletionKind::Table));
    assert_eq!(kind("COUNTROWS(FILTER(¦"), Some(CompletionKind::Table));
    assert_eq!(kind("SUM(¦"), Some(CompletionKind::Column));
    assert_eq!(kind("CALCULATE(¦"), Some(CompletionKind::Function));
}

#[test]
fn completion_is_empty_in_comments_literals_and_without_a_model() {
    for marked in [
        "-- SU¦",
        "1 + /* SU¦ */",
        "\"te¦xt\"",
        "\"abc¦",
        "12¦3",
        "1¦",
        "@par¦",
    ] {
        assert!(labels(marked).is_empty(), "{marked}");
    }
    // Without a model: functions, variables and keywords only.
    let (src, offset) = cursor("VAR s = 1 RETURN S¦");
    let c = complete(&src, offset, &Schema::unknown(), 0);
    assert!(!c.items.is_empty());
    assert!(c.items.iter().all(|i| matches!(
        i.kind,
        CompletionKind::Function | CompletionKind::Variable | CompletionKind::Keyword
    )));
    assert_eq!(c.items.first().map(|i| i.label.as_str()), Some("s"));
}

/// Every service answers at every offset of awkward text without panicking,
/// with spans inside the text on character boundaries.
#[test]
fn language_services_survive_every_offset() {
    let schema = Schema::from_model(&model());
    let texts = [
        "",
        "[",
        "'",
        "\"",
        "/*",
        "--",
        "SUM(",
        "VAR",
        "VAR x = ",
        "DEFINE MEASURE",
        "DEFINE MEASURE 'Prödüct'[Tötäl] = 1 EVALUATE {[Tötäl]}",
        "'Prödüct'[Ü] + [Tötä",
        "\"Ünï\" & 'Ä''b'[c]]d] /* ü */ -- é\n+ SUMX(FILTER(Sales, Sales[Amount] > 1), [x",
        "VAR a = (VAR b = 1 RETURN b) RETURN a, b)}) ((",
        "EVALUATE ORDER BY START AT ,,, @p dt\"2024",
        "|| | # ` 1e 1.2.3 .5 =>",
    ];
    for src in texts {
        let kind = SourceKind::detect(src);
        let _ = diagnostics(src, kind, &schema);
        for offset in 0..=src.len() + 2 {
            let _ = signature_help(src, offset);
            if let Some(h) = hover(src, offset, &schema) {
                assert!(h.span.end <= src.len(), "{src:?} @ {offset}");
            }
            let c = complete(src, offset, &schema, 50);
            assert!(
                c.span.start <= c.span.end && c.span.end <= src.len(),
                "{src:?} @ {offset}"
            );
            assert!(src.is_char_boundary(c.span.start) && src.is_char_boundary(c.span.end));
            assert!(c.items.len() <= 50);
        }
    }
}

fn reference_root() -> std::path::PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE").map_or_else(
        || std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference"),
        std::path::PathBuf::from,
    )
}

fn read_json(path: &std::path::Path) -> Option<serde_json::Value> {
    if !path.exists() {
        println!("SKIP: {} missing", path.display());
        return None;
    }
    Some(serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap())
}

fn has_argument_count_error(query: &str) -> bool {
    diagnostics(query, SourceKind::detect(query), &Schema::unknown())
        .iter()
        .any(|d| d.code == DiagnosticCode::ArgumentCount)
}

/// The number after `marker` in a Desktop error message.
fn number_after(message: &str, marker: &str) -> Option<usize> {
    let rest = &message[message.find(marker)? + marker.len()..];
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// Power BI Desktop's verdict on every catalog function called with too
/// few, the fewest, the most and too many arguments
/// (`reference/groundtruth/recordings/dax-arity.json`, generated by
/// `scripts/groundtruth/gen_corpus.py`; ADR-0011): `argumentCount` fires
/// exactly where Desktop rejects the argument count, and the catalog's
/// minimum and maximum are the ones Desktop states.
#[test]
fn catalog_argument_counts_match_power_bi_desktop() {
    let root = reference_root().join("groundtruth");
    let (Some(corpus), Some(recording)) = (
        read_json(&root.join("corpus/dax-arity.json")),
        read_json(&root.join("recordings/dax-arity.json")),
    ) else {
        return;
    };
    let queries: std::collections::HashMap<&str, (&str, &str, usize)> = corpus["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| {
            let n = usize::try_from(c["args"].as_u64().unwrap()).unwrap();
            let entry = (
                c["query"].as_str().unwrap(),
                c["function"].as_str().unwrap(),
                n,
            );
            (c["id"].as_str().unwrap(), entry)
        })
        .collect();
    let (mut accepted, mut rejected, mut inconclusive) = (0, 0, 0);
    let mut wrong = Vec::new();
    for rec in recording["cases"].as_array().unwrap() {
        let id = rec["id"].as_str().unwrap();
        // Stale or changed cases are `cargo xtask groundtruth`'s business.
        let Some(&(query, name, n)) = queries.get(id) else {
            continue;
        };
        if rec["query"] != query {
            continue;
        }
        let f = pbi_dax::service::catalog::function(name).unwrap();
        let flagged = has_argument_count_error(query);
        let Some(error) = rec["error"].as_str() else {
            accepted += 1;
            if flagged || !f.accepts(n) {
                wrong.push(format!("{id}: Desktop accepts {n} arguments"));
            }
            continue;
        };
        let min = number_after(error, "minimum argument count for the function is ");
        let max = number_after(error, "maximum argument count for the function is ");
        let other = error.contains("Maximum argument count of") || error.contains(" only takes ");
        if min.is_none() && max.is_none() && !other {
            inconclusive += 1;
            continue;
        }
        rejected += 1;
        if !flagged || f.accepts(n) {
            wrong.push(format!("{id}: Desktop rejects {n} arguments: {error}"));
        }
        if min.is_some_and(|m| m != f.min_args) || max.is_some_and(|m| Some(m) != f.max_args) {
            wrong.push(format!(
                "{id}: catalog {}..{:?}, Desktop: {error}",
                f.min_args, f.max_args
            ));
        }
    }
    println!("dax-arity: {accepted} accepted, {rejected} rejected, {inconclusive} inconclusive");
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert!(accepted > 0 && rejected > 0);
}

/// No query Power BI Desktop ran without an error, in any recording, gets
/// an `argumentCount` diagnostic.
#[test]
fn queries_desktop_runs_have_no_argument_count_errors() {
    let dir = reference_root().join("groundtruth/recordings");
    if !dir.is_dir() {
        println!("SKIP: {} missing", dir.display());
        return;
    }
    let mut checked = 0;
    let mut wrong = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let recording = read_json(&path).unwrap();
        for case in recording["cases"].as_array().into_iter().flatten() {
            let (Some(query), None) = (case["query"].as_str(), case.get("error")) else {
                continue;
            };
            checked += 1;
            if has_argument_count_error(query) {
                wrong.push(format!("{}: {}", path.display(), case["id"]));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    assert!(checked > 0);
}
