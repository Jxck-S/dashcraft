#![allow(clippy::unwrap_used, clippy::expect_used)]
//! One or more cases per supported DAX function, on a small hand-built
//! model (`Sales` → `Product`, with an unmatched product key so `Product`
//! has a blank row), plus BLANK semantics, cycles, depth and cancellation.

mod common;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use common::data::{batch, datetimes, floats, ints, strings, texts};
use pbi_dax::{
    DaxError, EvalContext, compute_calculated_column, evaluate_measure, evaluate_scalar,
    evaluate_table, supported_functions,
};
use pbi_engine::{ColumnFilter, Engine, FilterContext, Value};
use pbi_model::{Column, ColumnRef, DataType, Measure, Model, Relationship, Table};

fn model() -> Model {
    let mut product = Table::new("Product");
    product.columns = vec![
        Column::data("ProductKey", DataType::String),
        Column::data("Name", DataType::String),
        Column::data("Color", DataType::String),
        Column::data("Category", DataType::String),
    ];
    product.measures = vec![Measure::new("Product Count", "COUNTROWS(Product)")];
    let mut sales = Table::new("Sales");
    sales.columns = vec![
        Column::data("Id", DataType::Int64),
        Column::data("ProductKey", DataType::String),
        Column::data("Qty", DataType::Int64),
        Column::data("Price", DataType::Double),
        Column::data("Date", DataType::DateTime),
        Column::calculated("Line", DataType::Double, "Sales[Qty] * [Price]"),
    ];
    let mut measures = vec![
        Measure::new("Total Qty", "SUM(Sales[Qty])"),
        Measure::new("Rows", "COUNTROWS(Sales)"),
        Measure::new(
            "Red Qty",
            "CALCULATE([Total Qty], Product[Color] = \"Red\")",
        ),
        Measure::new("Cycle A", "[Cycle B] + 1"),
        Measure::new("Cycle B", "[Cycle A]"),
        Measure::new("Self", "[Self] * 2"),
        Measure::new("Broken", "1 +"),
    ];
    for i in 0..300 {
        measures.push(Measure::new(
            format!("Deep {i}"),
            format!("[Deep {}] + 1", i + 1),
        ));
    }
    measures.push(Measure::new("Deep 300", "0"));
    sales.measures = measures;
    Model {
        tables: vec![product, sales],
        relationships: vec![Relationship::new(
            "r",
            ColumnRef::new("Sales", "ProductKey"),
            ColumnRef::new("Product", "ProductKey"),
        )],
        ..Model::default()
    }
}

struct Fx {
    engine: Engine,
    model: Model,
}

fn fx() -> Fx {
    let model = model();
    let mut engine = Engine::new(&model);
    engine
        .load_table(
            "Product",
            batch(vec![
                ("ProductKey", texts(&["P1", "P2", "P3", "P4"])),
                ("Name", texts(&["Apple", "Banana", "Carrot", "Cherry"])),
                ("Color", texts(&["Red", "Yellow", "Orange", "Red"])),
                ("Category", texts(&["Fruit", "Fruit", "Veg", "Fruit"])),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Sales",
            batch(vec![
                (
                    "Id",
                    ints(&[Some(1), Some(2), Some(3), Some(4), Some(5), Some(6)]),
                ),
                (
                    "ProductKey",
                    strings(&[
                        Some("P1"),
                        Some("P1"),
                        Some("P2"),
                        Some("P3"),
                        Some("P9"),
                        Some("P2"),
                    ]),
                ),
                (
                    "Qty",
                    ints(&[Some(3), Some(2), Some(5), Some(1), Some(4), None]),
                ),
                (
                    "Price",
                    floats(&[
                        Some(1.5),
                        Some(1.5),
                        Some(0.25),
                        Some(2.0),
                        Some(3.0),
                        Some(0.5),
                    ]),
                ),
                (
                    "Date",
                    datetimes(&[
                        Some((2024, 1, 5, 0, 0, 0)),
                        Some((2024, 2, 10, 0, 0, 0)),
                        Some((2024, 1, 20, 0, 0, 0)),
                        Some((2024, 3, 1, 0, 0, 0)),
                        Some((2024, 3, 15, 0, 0, 0)),
                        None,
                    ]),
                ),
            ]),
        )
        .unwrap();
    let line = compute_calculated_column(&engine.snapshot(), &model, "Sales", "Line").unwrap();
    engine.add_column("Sales", "Line", line).unwrap();
    Fx { engine, model }
}

impl Fx {
    fn eval_in(&self, src: &str, filter: &FilterContext) -> Value {
        evaluate_scalar(&self.engine.snapshot(), &self.model, src, filter)
            .unwrap_or_else(|e| panic!("{src}: {e}"))
    }

    fn check(&self, cases: &[(&str, Value)]) {
        self.check_in(cases, &FilterContext::new());
    }

    fn check_in(&self, cases: &[(&str, Value)], filter: &FilterContext) {
        for (src, expected) in cases {
            assert_eq!(&self.eval_in(src, filter), expected, "{src}");
        }
    }

    fn err(&self, src: &str) -> DaxError {
        evaluate_scalar(
            &self.engine.snapshot(),
            &self.model,
            src,
            &FilterContext::new(),
        )
        .expect_err(src)
    }
}

fn s(v: &str) -> Value {
    Value::from(v)
}

fn i(v: i64) -> Value {
    Value::Integer(v)
}

fn d(v: f64) -> Value {
    Value::Double(v)
}

fn b(v: bool) -> Value {
    Value::Boolean(v)
}

fn logical_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("IF(1 > 2, \"a\", \"b\")", s("b")),
        ("IF(FALSE(), 1)", Value::Blank),
        ("IF(BLANK(), 1, 2)", i(2)),
        ("SWITCH(2, 1, \"one\", 2, \"two\", \"other\")", s("two")),
        ("SWITCH(TRUE(), 1 > 2, \"a\", \"else\")", s("else")),
        ("SWITCH(9, 1, \"one\")", Value::Blank),
        ("COALESCE(BLANK(), BLANK(), 3)", i(3)),
        ("ISBLANK(BLANK())", b(true)),
        ("ISBLANK(0)", b(false)),
        ("BLANK()", Value::Blank),
        ("TRUE()", b(true)),
        ("FALSE()", b(false)),
        ("NOT(1 = 1)", b(false)),
        ("NOT(BLANK())", b(true)),
        ("AND(TRUE(), BLANK())", b(false)),
        ("OR(FALSE(), BLANK())", b(false)),
        ("OR(BLANK(), BLANK())", Value::Blank),
        ("IFERROR(VALUE(\"abc\"), -1)", i(-1)),
        ("ISERROR(VALUE(\"abc\"))", b(true)),
        ("ISNUMBER(1.5)", b(true)),
        ("ISTEXT(\"a\")", b(true)),
        ("ISLOGICAL(TRUE())", b(true)),
    ]
}

fn math_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("DIVIDE(10, 4)", d(2.5)),
        ("DIVIDE(1, 0)", Value::Blank),
        ("DIVIDE(1, 0, -1)", i(-1)),
        ("DIVIDE(1, BLANK(), 9)", i(9)),
        ("DIVIDE(BLANK(), 5)", Value::Blank),
        ("ABS(-3)", i(3)),
        ("INT(-2.5)", i(-3)),
        ("ROUND(2.345, 2)", d(2.35)),
        ("ROUND(1.005, 2)", d(1.01)),
        ("ROUNDDOWN(2.3, 1)", d(2.3)),
        ("ROUND(1234, -2)", i(1200)),
        ("ROUNDUP(1.21, 1)", d(1.3)),
        ("ROUNDDOWN(1.29, 1)", d(1.2)),
        ("TRUNC(-1.7)", d(-1.0)),
        ("MOD(7, 3)", i(1)),
        ("MOD(-7, 3)", i(2)),
        ("POWER(2, 10)", d(1024.0)),
        ("SQRT(16)", d(4.0)),
        ("SIGN(-5)", i(-1)),
    ]
}

fn text_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("UPPER(\"abc\")", s("ABC")),
        ("LOWER(\"ABC\")", s("abc")),
        ("LEN(\"héllo\")", i(5)),
        ("LEFT(\"abcdef\", 2)", s("ab")),
        ("RIGHT(\"abcdef\", 3)", s("def")),
        ("MID(\"abcdef\", 2, 3)", s("bcd")),
        ("TRIM(\"  a   b  \")", s("a b")),
        ("CONCATENATE(\"a\", 1)", s("a1")),
        ("SUBSTITUTE(\"a-b-c\", \"-\", \"+\")", s("a+b+c")),
        ("SUBSTITUTE(\"a-b-c\", \"-\", \"+\", 2)", s("a-b+c")),
        ("CONTAINSSTRING(\"Web Sessions\", \"SESSION\")", b(true)),
        ("CONTAINSSTRING(\"abc\", \"a?c\")", b(true)),
        ("CONTAINSSTRING(\"abc\", \"x*\")", b(false)),
        ("CONTAINSSTRINGEXACT(\"abc\", \"B\")", b(false)),
        ("EXACT(\"a\", \"A\")", b(false)),
        ("VALUE(\"12.5\")", d(12.5)),
        ("FORMAT(5 / 11, \"0.0%;-0.0%;0.0%\")", s("45.5%")),
        ("FORMAT(1234567, \"#,0\")", s("1,234,567")),
        (
            "FORMAT(-1234.5, \"\\$#,0.00;(\\$#,0.00)\")",
            s("($1,234.50)"),
        ),
        ("FORMAT(BLANK(), \"0\")", s("")),
        ("FORMAT(12345.67, \"Standard\")", s("12,345.67")),
        (
            "FORMAT(DATE(2026, 10, 21) + 0.5, \"yyyy-mm-dd hh:nn\")",
            s("2026-10-21 12:00"),
        ),
        ("FORMAT(MAX(Sales[Date]), \"Short Date\")", s("3/15/2024")),
        ("FORMAT(3, BLANK())", s("3")),
        ("\"a\" & BLANK() & 1", s("a1")),
    ]
}

fn date_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("DATE(2024, 2, 30)", Value::date(2024, 3, 1).unwrap()),
        ("YEAR(DATE(2024, 5, 6))", i(2024)),
        ("MONTH(DATE(2024, 5, 6))", i(5)),
        ("DAY(DATE(2024, 5, 6))", i(6)),
        ("HOUR(dt\"2024-01-01T13:45:10\")", i(13)),
        ("MINUTE(dt\"2024-01-01T13:45:10\")", i(45)),
        ("SECOND(dt\"2024-01-01T13:45:10\")", i(10)),
        ("YEAR(NOW()) >= 2024", b(true)),
        ("TODAY() <= NOW()", b(true)),
        ("YEAR(UTCNOW()) >= 2024", b(true)),
        ("UTCTODAY() <= UTCNOW()", b(true)),
        ("MAX(Sales[Date])", Value::date(2024, 3, 15).unwrap()),
    ]
}

fn aggregate_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("SUM(Sales[Qty])", i(15)),
        ("SUM(Sales[Line])", d(22.75)),
        ("MIN(Sales[Qty])", i(1)),
        ("MAX(Sales[Price])", d(3.0)),
        ("MAX(1, 2)", i(2)),
        ("MIN(BLANK(), -1)", i(-1)),
        ("MAX(-1, BLANK())", i(0)),
        ("AVERAGE(Sales[Qty])", d(3.0)),
        ("COUNT(Sales[Qty])", i(5)),
        ("COUNTA(Sales[Date])", i(5)),
        ("COUNTBLANK(Sales[Qty])", i(1)),
        ("COUNTROWS(Sales)", i(6)),
        ("DISTINCTCOUNT(Sales[ProductKey])", i(4)),
        ("DISTINCTCOUNT(Product[Color])", i(4)),
        ("DISTINCTCOUNTNOBLANK(Product[Color])", i(3)),
        ("SUMX(Sales, Sales[Qty] * Sales[Price])", d(22.75)),
        ("COUNTX(Sales, Sales[Qty])", i(5)),
        ("COUNTAX(Sales, Sales[Date])", i(5)),
        ("MINX(Sales, Sales[Price])", d(0.25)),
        ("MAXX(Product, Product[Name])", s("Cherry")),
        ("AVERAGEX(Sales, Sales[Price])", d(8.75 / 6.0)),
        (
            "CONCATENATEX(Product, Product[Name], \", \", Product[Name], DESC)",
            s("Cherry, Carrot, Banana, Apple"),
        ),
    ]
}

fn context_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("CALCULATE(SUM(Sales[Qty]), Product[Color] = \"Red\")", i(5)),
        ("[Red Qty]", i(5)),
        ("Sales[Red Qty] + [Rows]", i(11)),
        (
            "ISEMPTY(FILTER(Product, Product[Color] = \"Blue\"))",
            b(true),
        ),
        ("ISEMPTY(Product)", b(false)),
        ("HASONEVALUE(Product[Color])", b(false)),
        (
            "CALCULATE(HASONEVALUE(Product[Color]), Product[Name] = \"Apple\")",
            b(true),
        ),
        (
            "CALCULATE(SELECTEDVALUE(Product[Color]), Product[Name] = \"Apple\")",
            s("Red"),
        ),
        ("SELECTEDVALUE(Product[Color], \"many\")", s("many")),
        (
            "CALCULATE(ISFILTERED(Product[Color]), Product[Color] = \"Red\")",
            b(true),
        ),
        ("ISFILTERED(Product[Color])", b(false)),
        ("MAXX(Sales, RELATED(Product[Name]))", s("Carrot")),
        (
            "SUMX(Sales, COUNTROWS(FILTER(Sales, Sales[Qty] > EARLIER(Sales[Qty]))))",
            i(15),
        ),
        (
            "SUMX(Sales, COUNTROWS(FILTER(Sales, Sales[Qty] > EARLIEST(Sales[Qty]))))",
            i(15),
        ),
        (
            "LOOKUPVALUE(Product[Name], Product[ProductKey], \"P2\")",
            s("Banana"),
        ),
        (
            "LOOKUPVALUE(Product[Name], Product[ProductKey], \"PX\", \"none\")",
            s("none"),
        ),
        ("MAXX(Product, COUNTROWS(RELATEDTABLE(Sales)))", i(2)),
        (
            "COUNTROWS(CALCULATETABLE(Sales, Product[Category] = \"Fruit\"))",
            i(4),
        ),
        ("MAXX(Product, [Total Qty])", i(5)),
        ("SUMX(VALUES(Product[Category]), [Total Qty])", i(15)),
        (
            "CALCULATE(SUM(Sales[Qty]), Product[Color] = \"Red\" || Product[Color] = \"Orange\")",
            i(6),
        ),
        (
            "CALCULATE(SUM(Sales[Qty]), Product[Color] = \"Red\", Product[Color] = \"Orange\")",
            Value::Blank,
        ),
        (
            "CALCULATE(SUM(Sales[Qty]), Product[Color] = \"Red\" && Product[Category] = \"Fruit\")",
            i(5),
        ),
        (
            "CALCULATE(SUM(Sales[Qty]), Product[Color] = \"Red\", KEEPFILTERS(Product[Name] = \"Banana\"))",
            Value::Blank,
        ),
        (
            "CALCULATE(SUM(Sales[Qty]), TREATAS({\"P2\"}, Product[ProductKey]))",
            i(5),
        ),
        ("CALCULATE([Total Qty], ISBLANK(Product[Color]))", i(4)),
    ]
}

fn table_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("COUNTROWS(FILTER(Sales, Sales[Qty] >= 3))", i(3)),
        ("COUNTROWS(ALL(Product[Color]))", i(4)),
        ("COUNTROWS(ALL(Product[Color], Product[Category]))", i(4)),
        ("COUNTROWS(ALLNOBLANKROW(Product[Color]))", i(3)),
        ("COUNTROWS(VALUES(Product[Category]))", i(3)),
        ("COUNTROWS(VALUES(Product))", i(5)),
        ("COUNTROWS(DISTINCT(Sales[ProductKey]))", i(4)),
        ("COUNTROWS(DISTINCT({1, 1, 2}))", i(2)),
        ("COUNTROWS(SUMMARIZE(Sales, Product[Category]))", i(3)),
        (
            "MAXX(SUMMARIZE(Sales, Product[Category], \"Q\", SUM(Sales[Qty])), [Q])",
            i(10),
        ),
        (
            "SUMX(SUMMARIZECOLUMNS(Product[Category], \"Q\", [Total Qty]), [Q])",
            i(15),
        ),
        (
            "COUNTROWS(SUMMARIZECOLUMNS(Product[Category], \"Q\", [Total Qty]))",
            i(3),
        ),
        (
            "COUNTROWS(SUMMARIZECOLUMNS(Product[Category], FILTER(ALL(Product[Color]), Product[Color] = \"Red\"), \"Q\", [Total Qty]))",
            i(1),
        ),
        (
            "COUNTROWS(SELECTCOLUMNS(Product, \"N\", Product[Name]))",
            i(4),
        ),
        ("MAXX(ADDCOLUMNS(Product, \"Q\", [Total Qty]), [Q])", i(5)),
        (
            "CONCATENATEX(TOPN(2, Product, Product[Name], ASC), Product[Name], \",\", Product[Name], ASC)",
            s("Apple,Banana"),
        ),
        ("COUNTROWS(TOPN(1, Product, Product[Category], ASC))", i(3)),
        ("COUNTROWS(TOPN(0, Product, Product[Name]))", Value::Blank),
        ("COUNTROWS(UNION({1, 2}, {2, 3}))", i(4)),
        ("COUNTROWS(INTERSECT({1, 2, 2}, {2, 3}))", i(2)),
        ("COUNTROWS(EXCEPT(VALUES(Product[Color]), {\"Red\"}))", i(3)),
        ("COUNTROWS(ROW(\"a\", 1, \"b\", \"x\"))", i(1)),
        (
            "SUMX(DATATABLE(\"Name\", STRING, \"Value\", INTEGER, {{\"a\", 1}, {\"b\", 2}}), [Value])",
            i(3),
        ),
        (
            "COUNTROWS(CROSSJOIN(VALUES(Product[Category]), {1, 2}))",
            i(6),
        ),
        ("SUMX(GENERATESERIES(1, 4), [Value])", i(10)),
        ("COUNTROWS(KEEPFILTERS(Product))", i(4)),
        ("2 IN {1, 2}", b(true)),
        ("(\"A\", 1) IN {(\"a\", 1), (\"b\", 2)}", b(true)),
        ("BLANK() IN {0}", b(false)),
        ("{5}", i(5)),
        (
            "VAR t = FILTER(Product, Product[Color] = \"Red\") RETURN COUNTROWS(t)",
            i(2),
        ),
        (
            "VAR t = FILTER(ALL(Product[Color]), Product[Color] = \"Red\") RETURN CALCULATE([Total Qty], t)",
            i(5),
        ),
    ]
}

fn date(y: i64, m: u32, dd: u32) -> Value {
    Value::date(y, m, dd).unwrap()
}

fn date_more_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("EOMONTH(DATE(2024, 1, 15), 1)", date(2024, 2, 29)),
        ("EOMONTH(BLANK(), 1)", Value::Blank),
        ("EDATE(DATE(2024, 1, 31), 1)", date(2024, 2, 29)),
        ("EDATE(DATE(2024, 3, 31), -13)", date(2023, 2, 28)),
        ("DATEDIFF(DATE(2024, 1, 31), DATE(2024, 3, 1), MONTH)", i(2)),
        ("DATEDIFF(DATE(2024, 1, 31), DATE(2024, 3, 1), DAY)", i(30)),
        (
            "DATEDIFF(DATE(2024, 3, 1), DATE(2023, 12, 31), YEAR)",
            i(-1),
        ),
        ("DATEDIFF(DATE(2024, 1, 6), DATE(2024, 1, 7), WEEK)", i(1)),
        (
            "DATEDIFF(DATE(2024, 3, 31), DATE(2024, 4, 1), QUARTER)",
            i(1),
        ),
        (
            "DATEDIFF(dt\"2024-01-01T10:00:00\", dt\"2024-01-01T12:30:00\", MINUTE)",
            i(150),
        ),
        (
            "DATEDIFF(dt\"2024-01-01T10:59:59\", dt\"2024-01-01T11:00:00\", HOUR)",
            i(1),
        ),
        (
            "DATEDIFF(dt\"2024-01-01T10:00:00\", dt\"2024-01-01T10:00:09\", SECOND)",
            i(9),
        ),
        ("WEEKDAY(DATE(2024, 1, 7))", i(1)),
        ("WEEKDAY(DATE(2024, 1, 7), 2)", i(7)),
        ("WEEKDAY(DATE(2024, 1, 8), 3)", i(0)),
        ("WEEKNUM(DATE(2024, 1, 7))", i(2)),
        ("WEEKNUM(DATE(2024, 1, 7), 2)", i(1)),
        ("WEEKNUM(DATE(2024, 12, 30), 21)", i(1)),
        ("QUARTER(DATE(2024, 5, 6))", i(2)),
        ("DATEVALUE(\"2024-03-05\")", date(2024, 3, 5)),
        ("DATEVALUE(\"3/5/2024 10:30\")", date(2024, 3, 5)),
        (
            "TIMEVALUE(\"1:30 PM\")",
            Value::datetime(1899, 12, 30, 13, 30, 0).unwrap(),
        ),
        (
            "TIME(1, 30, 15)",
            Value::datetime(1899, 12, 30, 1, 30, 15).unwrap(),
        ),
        (
            "COUNTROWS(CALENDAR(DATE(2024, 1, 1), DATE(2024, 12, 31)))",
            i(366),
        ),
        ("COUNTROWS(CALENDARAUTO())", i(366)),
        ("MINX(CALENDARAUTO(6), [Date])", date(2023, 7, 1)),
    ]
}

fn stats_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("MEDIAN(Sales[Qty])", d(3.0)),
        ("MEDIANX(Sales, Sales[Price])", d(1.5)),
        ("MEDIAN(Sales[Date])", date(2024, 2, 10)),
        ("PERCENTILE.INC(Sales[Qty], 0.25)", d(2.0)),
        ("PERCENTILE.EXC(Sales[Qty], 0.5)", d(3.0)),
        ("PERCENTILEX.INC(Sales, Sales[Qty], 1)", d(5.0)),
        ("PERCENTILEX.EXC(Sales, Sales[Qty], 0.25)", d(1.5)),
        ("STDEV.P(Sales[Qty])", d(2f64.sqrt())),
        ("STDEV.S(Sales[Qty])", d(2.5f64.sqrt())),
        ("VAR.P(Sales[Qty])", d(2.0)),
        ("VAR.S(Sales[Qty])", d(2.5)),
        ("STDEVX.P(Sales, Sales[Qty])", d(2f64.sqrt())),
        ("STDEVX.S(Sales, Sales[Qty])", d(2.5f64.sqrt())),
        ("VARX.P(Sales, Sales[Qty])", d(2.0)),
        ("VARX.S(Sales, Sales[Qty])", d(2.5)),
        (
            "VARX.S(FILTER(Sales, Sales[Qty] = 3), Sales[Qty])",
            Value::Blank,
        ),
        ("PRODUCT(Sales[Qty])", i(120)),
        ("PRODUCTX(Sales, Sales[Price])", d(1.6875)),
        ("PRODUCTX(FILTER(Sales, FALSE()), Sales[Qty])", Value::Blank),
        // Totals per name: Apple 5, Banana 5, Carrot 1, Cherry BLANK, and
        // 4 for the blank row (unmatched sales).
        (
            "CALCULATE(RANKX(ALL(Product[Name]), [Total Qty]), Product[Name] = \"Banana\")",
            i(1),
        ),
        (
            "CALCULATE(RANKX(ALL(Product[Name]), [Total Qty]), Product[Name] = \"Carrot\")",
            i(4),
        ),
        (
            "CALCULATE(RANKX(ALL(Product[Name]), [Total Qty], , DESC, DENSE), Product[Name] = \"Carrot\")",
            i(3),
        ),
        (
            "CALCULATE(RANKX(ALL(Product[Name]), [Total Qty], , ASC), Product[Name] = \"Carrot\")",
            i(2),
        ),
        ("RANKX(ALL(Product[Name]), [Total Qty], 4)", i(3)),
        ("RANKX(ALL(Product[Name]), [Total Qty], 4, 1, SKIP)", i(3)),
        ("RANK.EQ(4, Sales[Qty])", i(2)),
        ("RANK.EQ(4, Sales[Qty], ASC)", i(4)),
        ("RANK.EQ(99, Sales[Qty])", Value::Blank),
        // TOPN keeps ties at the boundary and sorts on several keys.
        (
            "COUNTROWS(TOPN(1, VALUES(Product[Name]), [Total Qty]))",
            i(2),
        ),
        (
            "CONCATENATEX(TOPN(1, VALUES(Product[Name]), [Total Qty], DESC, Product[Name], DESC), Product[Name])",
            s("Banana"),
        ),
        (
            "COUNTROWS(TOPN(2, VALUES(Product[Name]), [Total Qty], ASC))",
            i(2),
        ),
        ("COUNTROWS(TOPN(-1, Product, Product[Name]))", Value::Blank),
    ]
}

fn window_cases() -> Vec<(&'static str, Value)> {
    vec![
        (
            "CALCULATE(CALCULATE([Total Qty], OFFSET(-1, ALL(Product[Name]), ORDERBY(Product[Name]))), Product[Name] = \"Banana\")",
            i(5),
        ),
        (
            "CALCULATE(CALCULATE([Total Qty], OFFSET(1, ALL(Product[Name]), ORDERBY(Product[Name]))), Product[Name] = \"Banana\")",
            i(1),
        ),
        (
            "COUNTROWS(OFFSET(-1, ALL(Product[Name]), ORDERBY(Product[Name])))",
            Value::Blank,
        ),
        (
            "INDEX(1, ALL(Product[Name]), ORDERBY(Product[Name], DESC))",
            s("Cherry"),
        ),
        (
            "INDEX(-1, ALL(Product[Name]), ORDERBY(Product[Name]))",
            s("Cherry"),
        ),
        (
            "COUNTROWS(WINDOW(1, ABS, -1, ABS, ALL(Product[Name]), ORDERBY(Product[Name])))",
            i(5),
        ),
        (
            "CALCULATE(SUMX(WINDOW(-1, REL, 0, REL, ALL(Product[Name]), ORDERBY(Product[Name])), [Total Qty]), Product[Name] = \"Banana\")",
            i(10),
        ),
        (
            "CALCULATE(CONCATENATEX(INDEX(1, ALL(Product[Category], Product[Name]), ORDERBY(Product[Name], DESC), PARTITIONBY(Product[Category])), Product[Name]), Product[Name] = \"Apple\")",
            s("Cherry"),
        ),
        (
            "CALCULATE(CONCATENATEX(OFFSET(1, ALL(Product[Category], Product[Name]), ORDERBY(Product[Name]), PARTITIONBY(Product[Category]), MATCHBY(Product[Category], Product[Name])), Product[Name]), Product[Name] = \"Apple\")",
            s("Banana"),
        ),
        (
            "CALCULATE(ROWNUMBER(ALL(Product[Name]), ORDERBY(Product[Name])), Product[Name] = \"Banana\")",
            i(3),
        ),
        (
            "CALCULATE(RANK(DENSE, ALL(Product[Name]), ORDERBY([Total Qty], DESC)), Product[Name] = \"Carrot\")",
            i(3),
        ),
        (
            "CALCULATE(RANK(SKIP, ALL(Product[Name]), ORDERBY([Total Qty], DESC)), Product[Name] = \"Carrot\")",
            i(4),
        ),
        // No relation: the ALLSELECTED values of the ORDERBY column.
        (
            "CALCULATE(CALCULATE([Total Qty], OFFSET(1, ORDERBY(Product[Name]))), Product[Name] = \"Apple\")",
            i(5),
        ),
    ]
}

fn misc_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("SWITCH(TRUE(), 1 > 2, \"a\", 2 > 1, \"b\", \"c\")", s("b")),
        ("ISNONTEXT(1)", b(true)),
        ("ISEVEN(4)", b(true)),
        ("ISODD(-3)", b(true)),
        ("CONVERT(\"12\", INTEGER)", i(12)),
        ("CONVERT(2.5, INTEGER)", i(3)),
        ("CONVERT(1.5, STRING)", s("1.5")),
        ("CONVERT(1, BOOLEAN)", b(true)),
        ("CONVERT(1.23456, CURRENCY)", Value::Decimal(12_346)),
        ("CONVERT(\"2024-01-02\", DATETIME)", date(2024, 1, 2)),
        ("CONVERT(DATE(1900, 1, 1), DOUBLE)", d(2.0)),
        ("CURRENCY(1.23456)", Value::Decimal(12_346)),
        ("FIXED(1234.567, 1)", s("1,234.6")),
        ("FIXED(1234.567, 1, TRUE())", s("1234.6")),
        ("FIXED(1234.5)", s("1,234.50")),
        ("FIXED(1250, -2)", s("1,300")),
        ("UNICHAR(65)", s("A")),
        ("UNICODE(\"é\")", i(233)),
        ("REPT(\"ab\", 3)", s("ababab")),
        ("REPT(\"ab\", 0)", Value::Blank),
        ("SEARCH(\"b?d\", \"ABCD\")", i(2)),
        ("SEARCH(\"C*\", \"abcabc\", 4)", i(6)),
        ("SEARCH(\"x\", \"abc\", 1, -1)", i(-1)),
        ("FIND(\"B\", \"aBcB\", 3)", i(4)),
        ("FIND(\"b\", \"aBc\", 1, 0)", i(0)),
        ("REPLACE(\"abcdef\", 2, 3, \"X\")", s("aXef")),
        ("COMBINEVALUES(\"-\", \"a\", 1)", s("a-1")),
        ("CONTAINS(Product, Product[Color], \"red\")", b(true)),
        (
            "CONTAINS(Product, Product[Color], \"Red\", Product[Category], \"Veg\")",
            b(false),
        ),
        ("CONTAINSROW({1, 2}, 2)", b(true)),
        ("CONTAINSROW(VALUES(Product[Color]), \"RED\")", b(true)),
        ("PATHITEM(\"1|2|3\", 2)", s("2")),
        ("PATHITEM(\"1|2|3\", 2, 1)", i(2)),
        ("PATHITEM(\"1|2|3\", 9)", Value::Blank),
        ("PATHITEMREVERSE(\"1|2|3\", 1)", s("3")),
        ("PATHLENGTH(\"1|2|3\")", i(3)),
        ("PATHCONTAINS(\"1|2|3\", \"2\")", b(true)),
        // No effective user configured: empty text, CUSTOMDATA blank (see
        // tests/model_features.rs for "view as" identities).
        ("USERPRINCIPALNAME()", s("")),
        ("USERNAME() & \"|\" & USEROBJECTID()", s("|")),
        ("ISBLANK(CUSTOMDATA())", b(true)),
        (
            "COUNTROWS(GENERATE(VALUES(Product[Category]), CALCULATETABLE(VALUES(Product[Color]))))",
            i(4),
        ),
        (
            "COUNTROWS(GENERATE(VALUES(Product[Category]), FILTER(VALUES(Product[Color]), FALSE())))",
            Value::Blank,
        ),
        (
            "COUNTROWS(GENERATEALL(VALUES(Product[Category]), FILTER(VALUES(Product[Color]), FALSE())))",
            i(3),
        ),
        (
            "COUNTROWS(NATURALINNERJOIN(SELECTCOLUMNS(Sales, \"K\", Sales[ProductKey] & \"\"), SELECTCOLUMNS(Product, \"K\", Product[ProductKey] & \"\", \"N\", Product[Name])))",
            i(5),
        ),
        (
            "COUNTROWS(NATURALLEFTOUTERJOIN(SELECTCOLUMNS(Sales, \"K\", Sales[ProductKey] & \"\"), SELECTCOLUMNS(Product, \"K\", Product[ProductKey] & \"\", \"N\", Product[Name])))",
            i(6),
        ),
        (
            "SUMX(GROUPBY(Sales, Product[Category], \"Q\", SUMX(CURRENTGROUP(), Sales[Qty])), [Q])",
            i(15),
        ),
        ("COUNTROWS(GROUPBY(Sales, Product[Category]))", i(3)),
        (
            "MAXX(GROUPBY(Sales, Sales[ProductKey], \"Q\", SUMX(CURRENTGROUP(), Sales[Qty])), [Q])",
            i(5),
        ),
        (
            "COUNTROWS(SUMMARIZECOLUMNS(Product[Name], \"Q\", [Total Qty], \"One\", IGNORE(1)))",
            i(4),
        ),
        (
            "COUNTROWS(SUMMARIZECOLUMNS(Product[Name], \"Q\", [Total Qty], \"One\", 1))",
            i(5),
        ),
        ("IGNORE(7)", i(7)),
    ]
}

/// `ALLSELECTED`, `ISINSCOPE`, `FILTERS`, `HASONEFILTER`, `ISCROSSFILTERED`
/// and `SUMMARIZECOLUMNS` subtotals.
fn visual_cases() -> Vec<(&'static str, Value)> {
    vec![
        (
            "MAXX(SUMMARIZECOLUMNS(Product[Category], TREATAS({\"Fruit\", \"Veg\"}, Product[Category]), \"Pct\", DIVIDE([Total Qty], CALCULATE([Total Qty], ALLSELECTED()))), [Pct])",
            d(10.0 / 11.0),
        ),
        (
            "MAXX(SUMMARIZECOLUMNS(Product[Category], TREATAS({\"Fruit\", \"Veg\"}, Product[Category]), \"N\", COUNTROWS(ALLSELECTED(Product[Category]))), [N])",
            i(2),
        ),
        (
            "MAXX(SUMMARIZECOLUMNS(Product[Color], TREATAS({\"Red\"}, Product[Color]), \"N\", COUNTROWS(ALLSELECTED(Product))), [N])",
            i(2),
        ),
        (
            "MAXX(SUMMARIZECOLUMNS(Product[Category], TREATAS({\"Fruit\", \"Veg\"}, Product[Category]), \"Q\", CALCULATE([Total Qty], ALLSELECTED(Product[Category]))), [Q])",
            i(11),
        ),
        ("COUNTROWS(ALLSELECTED(Product[Category]))", i(3)),
        ("CALCULATE([Total Qty], ALLSELECTED(Product))", i(15)),
        ("ISINSCOPE(Product[Color])", b(false)),
        (
            "COUNTROWS(FILTER(SUMMARIZECOLUMNS(Product[Category], \"S\", ISINSCOPE(Product[Category])), [S]))",
            i(3),
        ),
        (
            "COUNTROWS(SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Product[Category], \"CatTotal\", Product[Color], \"ColorTotal\"), \"Q\", [Total Qty]))",
            i(8),
        ),
        (
            "COUNTROWS(SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(ROLLUPGROUP(Product[Category], Product[Color]), \"T\"), \"Q\", [Total Qty]))",
            i(5),
        ),
        ("COUNTROWS(FILTERS(Product[Color]))", i(4)),
        (
            "CALCULATE(COUNTROWS(FILTERS(Product[Color])), Product[Color] IN {\"Red\", \"Orange\"}, Product[Category] = \"Fruit\")",
            i(2),
        ),
        (
            "CALCULATE(HASONEFILTER(Product[Color]), Product[Color] = \"Red\")",
            b(true),
        ),
        ("HASONEFILTER(Product[Color])", b(false)),
        (
            "CALCULATE(HASONEFILTER(Product[Color]), Product[Name] = \"Apple\")",
            b(false),
        ),
        (
            "CALCULATE(ISCROSSFILTERED(Sales[Qty]), Product[Color] = \"Red\")",
            b(true),
        ),
        (
            "CALCULATE(ISCROSSFILTERED(Product), Sales[Qty] = 3)",
            b(false),
        ),
        (
            "CALCULATE(ISCROSSFILTERED(Product[Name]), Product[Color] = \"Red\")",
            b(true),
        ),
        ("ISCROSSFILTERED(Product[Name])", b(false)),
        (
            "CALCULATE(ISCROSSFILTERED(Product), FILTER(Sales, Sales[Qty] = 3))",
            b(true),
        ),
        (
            "CALCULATE(ISFILTERED(Product), ALLSELECTED(Product[Color]))",
            b(false),
        ),
    ]
}

/// Exact results of the math, bit, statistical, date and "other" functions
/// added with the function-library wave.
fn library_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("PI()", d(std::f64::consts::PI)),
        ("EXP(0)", d(1.0)),
        ("EXP(BLANK())", d(1.0)),
        ("LN(1)", d(0.0)),
        ("LOG(1000)", d(3.0)),
        ("LOG(8, 2)", d(3.0)),
        ("LOG10(100)", d(2.0)),
        ("SQRTPI(0)", d(0.0)),
        ("SIN(0)", d(0.0)),
        ("COS(0)", d(1.0)),
        ("TAN(0)", d(0.0)),
        ("ASIN(1) * 2 = PI()", b(true)),
        ("ACOS(1)", d(0.0)),
        ("ATAN(0)", d(0.0)),
        ("ACOT(0) * 2 = PI()", b(true)),
        ("SINH(0)", d(0.0)),
        ("COSH(0)", d(1.0)),
        ("TANH(0)", d(0.0)),
        ("ASINH(0)", d(0.0)),
        ("ACOSH(1)", d(0.0)),
        ("ATANH(0)", d(0.0)),
        ("FACT(5)", d(120.0)),
        ("FACT(0)", d(1.0)),
        ("FACT(3.9)", d(6.0)),
        ("COMBIN(8, 2)", i(28)),
        ("COMBIN(60, 30)", i(118_264_581_564_861_424)),
        ("COMBINA(4, 3)", i(20)),
        ("PERMUT(100, 3)", i(970_200)),
        ("GCD(24, 36)", i(12)),
        ("GCD(BLANK(), 4)", i(4)),
        ("LCM(4, 6)", i(12)),
        ("QUOTIENT(5, 2)", i(2)),
        ("QUOTIENT(-10, 3)", i(-3)),
        ("QUOTIENT(4.5, 3.1)", i(1)),
        ("EVEN(1.5)", i(2)),
        ("EVEN(-1)", i(-2)),
        ("ODD(0)", i(1)),
        ("ODD(-2)", i(-3)),
        ("ODD(3)", i(3)),
        ("MROUND(10, 3)", d(9.0)),
        ("MROUND(-10, -3)", d(-9.0)),
        ("MROUND(1.3, 0.2)", d(1.4)),
        ("MROUND(BLANK(), 2)", Value::Blank),
        ("CEILING(4.42, 0.05)", d(4.45)),
        ("CEILING(1.5, 0.1)", d(1.5)),
        ("CEILING(2.5, 1)", i(3)),
        ("CEILING(-2.5, 2)", i(-2)),
        ("CEILING(-2.5, -2)", i(-4)),
        ("CEILING(CURRENCY(1.23), 0.5)", Value::Decimal(15_000)),
        ("ISO.CEILING(-4.3)", d(-4.0)),
        ("ISO.CEILING(4.3, -2)", d(6.0)),
        ("ISO.CEILING(7, 2)", i(8)),
        ("FLOOR(10.8423, 0.1)", d(10.8)),
        ("FLOOR(2.9733, 0.1)", d(2.9)),
        ("FLOOR(-2.5, -2)", d(-2.0)),
        ("RANDBETWEEN(3, 3)", i(3)),
        ("RAND() >= 0 && RAND() < 1", b(true)),
        ("BITAND(13, 11)", i(9)),
        ("BITOR(9, 4)", i(13)),
        ("BITXOR(5, 3)", i(6)),
        ("BITLSHIFT(2, 3)", i(16)),
        ("BITLSHIFT(128, -1)", i(64)),
        ("BITRSHIFT(-16, 2)", i(-4)),
        ("IF.EAGER(1 > 2, \"a\", \"b\")", s("b")),
        ("IFERROR(ERROR(\"boom\"), 7)", i(7)),
        ("EVALUATEANDLOG(1 + 1, \"two\")", i(2)),
        ("USERCULTURE()", s("en-US")),
        ("ISAFTER(2, 1)", b(true)),
        ("ISAFTER(1, 1)", b(false)),
        ("ISONORAFTER(1, 1)", b(true)),
        ("ISAFTER(\"a\", \"b\", DESC)", b(true)),
        ("ISONORAFTER(1, 1, ASC, 2, 3, ASC)", b(false)),
        // Aggregations of the A family and the geometric mean.
        ("MAXA(Sales[Qty])", i(5)),
        ("MINA(Sales[Price])", d(0.25)),
        ("MAXA(Sales[Date])", date(2024, 3, 15)),
        ("MAXA(Product[Color])", i(0)),
        ("AVERAGEA(Sales[Qty])", d(3.0)),
        ("AVERAGEA(Product[Name])", d(0.0)),
        (
            "CALCULATE(MINA(Sales[Qty]), Product[Color] = \"Blue\")",
            Value::Blank,
        ),
        ("GEOMEANX(FILTER(Sales, FALSE()), Sales[Qty])", Value::Blank),
        // YEARFRAC (Excel reference examples) and NETWORKDAYS.
        (
            "YEARFRAC(DATE(2012, 1, 1), DATE(2012, 7, 30))",
            d(209.0 / 360.0),
        ),
        (
            "YEARFRAC(DATE(2012, 7, 30), DATE(2012, 1, 1), 1)",
            d(211.0 / 366.0),
        ),
        (
            "YEARFRAC(DATE(2012, 1, 1), DATE(2012, 7, 30), 2)",
            d(211.0 / 360.0),
        ),
        (
            "YEARFRAC(DATE(2012, 1, 1), DATE(2012, 7, 30), 3)",
            d(211.0 / 365.0),
        ),
        (
            "YEARFRAC(DATE(2012, 1, 31), DATE(2012, 3, 31), 4)",
            d(60.0 / 360.0),
        ),
        ("YEARFRAC(DATE(2011, 2, 28), DATE(2012, 2, 29))", d(1.0)),
        (
            "YEARFRAC(DATE(2011, 1, 31), DATE(2011, 2, 28))",
            d(28.0 / 360.0),
        ),
        (
            "YEARFRAC(DATE(2010, 6, 1), DATE(2013, 6, 1), 1)",
            d(1096.0 / (1461.0 / 4.0)),
        ),
        (
            "NETWORKDAYS(DATE(2022, 5, 28), DATE(2022, 5, 30), 1, {DATE(2022, 5, 30)})",
            i(0),
        ),
        ("NETWORKDAYS(DATE(2024, 1, 1), DATE(2024, 1, 31))", i(23)),
        ("NETWORKDAYS(DATE(2024, 1, 31), DATE(2024, 1, 1))", i(-23)),
        (
            "NETWORKDAYS(DATE(2024, 1, 1), DATE(2024, 1, 31), 11)",
            i(27),
        ),
        (
            "NETWORKDAYS(DATE(2024, 1, 1), DATE(2024, 1, 7), \"0000011\")",
            i(5),
        ),
        (
            "NETWORKDAYS(DATE(2024, 12, 23), DATE(2025, 1, 3), 1, {DATE(2024, 12, 25), DATE(2024, 12, 26), DATE(2025, 1, 1), DATE(2024, 12, 28)})",
            i(7),
        ),
        ("ISBLANK(NETWORKDAYS(BLANK(), BLANK()))", b(true)),
        // Existing date functions checked against the reference.
        ("EDATE(DATE(2024, 1, 31), 1.9)", date(2024, 2, 29)),
        ("EOMONTH(DATE(2008, 3, 3), 1.5)", date(2008, 5, 31)),
        ("ISBLANK(QUARTER(BLANK()))", b(true)),
        ("DATEDIFF(DATE(2019, 7, 1), DATE(2021, 12, 31), YEAR)", i(2)),
        (
            "DATEDIFF(DATE(2019, 7, 1), DATE(2021, 12, 31), QUARTER)",
            i(9),
        ),
        (
            "DATEDIFF(DATE(2019, 7, 1), DATE(2021, 12, 31), MONTH)",
            i(29),
        ),
        (
            "DATEDIFF(DATE(2019, 7, 1), DATE(2021, 12, 31), WEEK)",
            i(130),
        ),
        (
            "DATEDIFF(DATE(2019, 7, 1), DATE(2021, 12, 31), DAY)",
            i(914),
        ),
    ]
}

/// Results of the new numeric functions that are compared with a relative
/// tolerance of 1e-12 (closed forms, or values from an independent
/// implementation; see `tests/functions_library.rs` for more).
fn library_approx_cases() -> Vec<(&'static str, f64)> {
    let pi = std::f64::consts::PI;
    vec![
        ("SQRTPI(2)", (2.0 * pi).sqrt()),
        ("LOG(125, 5)", 3.0),
        ("LOG10(0.001)", -3.0),
        ("SIN(PI() / 6)", 0.5),
        ("COS(PI() / 3)", 0.5),
        ("TAN(PI() / 4)", 1.0),
        ("COT(PI() / 4)", 1.0),
        ("ACOS(0.5)", pi / 3.0),
        ("ASIN(0.5)", pi / 6.0),
        ("ATAN(1)", pi / 4.0),
        ("ACOT(1)", pi / 4.0),
        ("COTH(1)", 1.0 / 1f64.tanh()),
        ("ACOTH(2)", 0.5f64.atanh()),
        ("SINH(1)", 1f64.sinh()),
        ("DEGREES(PI())", 180.0),
        ("RADIANS(180)", pi),
        ("EXP(1)", std::f64::consts::E),
        ("LN(EXP(2))", 2.0),
        ("FACT(170)", 7.257_415_615_307_999e306),
        ("GEOMEAN(Sales[Qty])", 120f64.powf(0.2)),
        ("GEOMEANX(Sales, Sales[Price])", 1.6875f64.powf(1.0 / 6.0)),
        (
            "CALCULATE(GEOMEAN(Sales[Qty]), Product[Color] = \"Red\")",
            6f64.sqrt(),
        ),
        ("NORM.S.DIST(1.96, TRUE)", 0.975_002_104_851_779_5),
        ("NORM.S.DIST(0, FALSE)", 1.0 / (2.0 * pi).sqrt()),
        ("NORM.S.INV(0.975)", 1.959_963_984_540_054),
        ("NORM.DIST(42, 40, 1.5, TRUE)", 0.908_788_780_274_132_1),
        ("NORM.DIST(42, 40, 1.5, FALSE)", 0.109_340_049_783_995_77),
        ("NORM.INV(0.908789, 40, 1.5)", 42.000_002_009_566_16),
        ("EXPON.DIST(0.2, 10, TRUE)", 1.0 - (-2f64).exp()),
        ("EXPON.DIST(0.2, 10, FALSE)", 10.0 * (-2f64).exp()),
        ("POISSON.DIST(2, 5, FALSE)", (-5f64).exp() * 12.5),
        ("POISSON.DIST(2, 5, TRUE)", (-5f64).exp() * 18.5),
        ("BETA.DIST(2, 8, 10, TRUE, 1, 3)", 0.685_470_581_054_687_5),
        ("BETA.DIST(2, 8, 10, FALSE, 1, 3)", 1.483_764_648_437_500_2),
        (
            "BETA.INV(0.685470581, 8, 10, 1, 3)",
            1.999_999_999_963_142_8,
        ),
        ("CHISQ.DIST(3, 2, TRUE)", 1.0 - (-1.5f64).exp()),
        ("CHISQ.DIST(2, 3, FALSE)", 0.207_553_748_710_297_4),
        ("CHISQ.DIST.RT(18.307, 10)", 0.050_000_589_091_398_075),
        ("CHISQ.INV(0.6, 2)", -2.0 * 0.4f64.ln()),
        ("CHISQ.INV.RT(0.05, 10)", 18.307_038_053_275_143),
        ("T.DIST(60, 1, TRUE)", 0.5 + 60f64.atan() / pi),
        (
            "T.DIST(8, 3, FALSE)",
            6.0 * 3f64.sqrt() / (pi * 67.0 * 67.0),
        ),
        ("T.DIST.2T(1.959999998, 60)", 0.054_644_929_975_920_87),
        ("T.DIST.RT(1.959999998, 60)", 0.027_322_464_987_960_433),
        ("T.INV(0.75, 2)", (2.0f64 / 3.0).sqrt()),
        // Exact for ν = 10 (even-ν closed form of the t CDF).
        ("T.INV.2T(0.05, 10)", 2.228_138_851_986_274),
        ("CONFIDENCE.NORM(0.05, 2.5, 50)", 0.692_951_912_174_839_1),
        ("CONFIDENCE.T(0.05, 1, 50)", 0.284_196_855_495_729_9),
        // Inside iterators and CALCULATE.
        (
            "SUMX(GENERATESERIES(1, 3), NORM.S.DIST([Value], TRUE))",
            2.817_244_716_088_733_7,
        ),
        // LN(BLANK()) is LN(0), an error: skip the row without a quantity.
        (
            "SUMX(FILTER(Sales, NOT ISBLANK(Sales[Qty])), LN(Sales[Qty]))",
            120f64.ln(),
        ),
        (
            "CALCULATE(SUMX(Sales, EXP(Sales[Qty])), Product[Color] = \"Red\")",
            3f64.exp() + 2f64.exp(),
        ),
    ]
}

#[test]
fn library_functions() {
    let f = fx();
    f.check(&library_cases());
    for (src, want) in library_approx_cases() {
        let got = match f.eval_in(src, &FilterContext::new()) {
            Value::Double(x) => x,
            Value::Integer(n) => n as f64,
            other => panic!("{src}: expected a number, got {other:?}"),
        };
        assert!(
            (got - want).abs() <= 1e-12 * want.abs().max(1e-300),
            "{src}: {got:e} vs {want:e}"
        );
    }
}

#[test]
fn logical_functions() {
    fx().check(&logical_cases());
}

#[test]
fn math_functions() {
    fx().check(&math_cases());
}

#[test]
fn text_functions() {
    fx().check(&text_cases());
}

#[test]
fn date_functions() {
    fx().check(&date_cases());
}

#[test]
fn aggregate_and_iterator_functions() {
    fx().check(&aggregate_cases());
}

#[test]
fn context_functions() {
    fx().check(&context_cases());
}

#[test]
fn table_functions() {
    fx().check(&table_cases());
}

#[test]
fn modifiers_remove_outer_filters() {
    let f = fx();
    let red_fruit = FilterContext::new()
        .with(ColumnFilter::equals("Product", "Color", "Red"))
        .with(ColumnFilter::equals("Product", "Category", "Fruit"));
    f.check_in(
        &[
            ("SUM(Sales[Qty])", i(5)),
            ("CALCULATE(SUM(Sales[Qty]), REMOVEFILTERS(Product))", i(15)),
            ("CALCULATE(SUM(Sales[Qty]), ALL(Product))", i(15)),
            ("CALCULATE(SUM(Sales[Qty]), ALL(Sales))", i(15)),
            ("CALCULATE(SUM(Sales[Qty]), ALL())", i(15)),
            (
                "CALCULATE(SUM(Sales[Qty]), ALLEXCEPT(Product, Product[Category]))",
                i(10),
            ),
            (
                "CALCULATE(SUM(Sales[Qty]), Product[Color] = \"Yellow\")",
                i(5),
            ),
            ("COUNTROWS(Product)", i(2)),
            ("[Product Count]", i(2)),
            ("COUNTROWS(ALL(Product))", i(5)),
        ],
        &red_fruit,
    );
}

#[test]
fn blank_semantics() {
    fx().check(&[
        ("BLANK() + 1", i(1)),
        ("BLANK() - 1", i(-1)),
        ("BLANK() * 1", Value::Blank),
        ("BLANK() + BLANK()", Value::Blank),
        ("BLANK() / BLANK()", Value::Blank),
        ("5 / BLANK()", d(f64::INFINITY)),
        ("BLANK() = 0", b(true)),
        ("BLANK() == 0", b(false)),
        ("BLANK() = \"\"", b(true)),
        ("BLANK() && TRUE()", b(false)),
        ("BLANK() || TRUE()", b(true)),
        ("-BLANK()", Value::Blank),
        ("BLANK() & \"x\"", s("x")),
        ("\"22\" + 22", i(44)),
        ("1 / 2", d(0.5)),
        ("2 ^ 3", d(8.0)),
        ("\"abc\" = \"ABC\"", b(true)),
    ]);
}

#[test]
fn calculated_column_uses_row_context() {
    let f = fx();
    let snap = f.engine.snapshot();
    let values: Vec<Value> = (0..6)
        .map(|r| snap.value_at("Sales", "Line", r).unwrap())
        .collect();
    assert_eq!(
        values,
        vec![d(4.5), d(3.0), d(1.25), d(2.0), d(12.0), Value::Blank]
    );
}

#[test]
fn measure_cycles_and_errors() {
    let f = fx();
    let snap = f.engine.snapshot();
    let none = FilterContext::new();
    match evaluate_measure(&snap, &f.model, "Cycle A", &none) {
        Err(DaxError::CircularDependency { chain }) => {
            assert_eq!(chain, ["[Cycle A]", "[Cycle B]", "[Cycle A]"]);
        }
        other => panic!("expected a cycle, got {other:?}"),
    }
    assert!(matches!(
        evaluate_measure(&snap, &f.model, "[Self]", &none),
        Err(DaxError::CircularDependency { .. })
    ));
    assert!(matches!(
        evaluate_measure(&snap, &f.model, "Sales[Broken]", &none),
        Err(DaxError::Parse { .. })
    ));
    assert!(matches!(
        evaluate_measure(&snap, &f.model, "Deep 0", &none),
        Err(DaxError::DepthExceeded)
    ));
    assert_eq!(
        evaluate_measure(&snap, &f.model, "Deep 250", &none).unwrap(),
        i(50)
    );
    assert!(matches!(
        f.err("TOJSON(Product)"),
        DaxError::UnsupportedFunction { name } if name == "TOJSON"
    ));
    assert!(
        f.err("CALCULATE([Total Qty], SAMPLE(1, Product, Product[Name]))")
            .is_unsupported()
    );
    assert!(
        f.err("ORDERBY(Product[Name])")
            .to_string()
            .contains("OFFSET")
    );
    assert!(matches!(
        f.err("USERELATIONSHIP(Sales[ProductKey], Product[ProductKey])"),
        DaxError::Argument { .. }
    ));
    assert!(matches!(
        f.err("CALCULATE([Total Qty], USERELATIONSHIP(Product[Name], Sales[Qty]))"),
        DaxError::Argument { function, .. } if function == "USERELATIONSHIP"
    ));
    assert!(matches!(
        f.err("CALCULATE([Total Qty], CROSSFILTER(Sales[ProductKey], Product[ProductKey], Sideways))"),
        DaxError::Argument { function, .. } if function == "CROSSFILTER"
    ));
    assert!(matches!(f.err("CURRENTGROUP()"), DaxError::Argument { .. }));
    assert!(matches!(
        f.err("SEARCH(\"z\", \"abc\")"),
        DaxError::Evaluation { .. }
    ));
    assert!(matches!(f.err("UNICHAR(-1)"), DaxError::Argument { .. }));
    assert!(matches!(
        f.err("REPT(\"a\", -1)"),
        DaxError::Argument { .. }
    ));
    assert!(matches!(
        f.err("PERCENTILE.EXC(Sales[Qty], 0.01)"),
        DaxError::Argument { .. }
    ));
    assert!(matches!(
        f.err("DATEADD(Sales[Date], 1, WEEK)"),
        DaxError::Argument { .. }
    ));
    assert!(matches!(
        f.err("CALENDAR(DATE(2024, 2, 1), DATE(2024, 1, 1))"),
        DaxError::Argument { .. }
    ));
    assert!(matches!(
        f.err("COUNTROWS(NATURALINNERJOIN({1}, ROW(\"x\", 1)))"),
        DaxError::Argument { .. }
    ));
    assert!(matches!(
        f.err("SUMMARIZECOLUMNS(Product[Color], NONVISUAL(TREATAS({\"Red\"}, Product[Color])), \"Q\", [Total Qty])"),
        DaxError::UnsupportedFunction { .. }
    ));
    assert!(matches!(f.err("ALLSELECTED()"), DaxError::Argument { .. }));
    assert!(matches!(
        f.err("PATH(Product[Name], Product[Name])"),
        DaxError::Argument { .. }
    ));
    assert!(matches!(
        f.err("SUM(1)"),
        DaxError::Argument { function, .. } if function == "SUM"
    ));
    assert!(matches!(f.err("IF(1)"), DaxError::Argument { .. }));
    assert!(matches!(f.err("\"a\" + 1"), DaxError::Type { .. }));
    assert!(matches!(f.err("\"a\" = 1"), DaxError::Type { .. }));
    assert!(matches!(
        f.err("VALUES(Product[Color])"),
        DaxError::Type { .. }
    ));
    assert!(matches!(f.err("Product"), DaxError::Type { .. }));
    assert!(matches!(f.err("[Qty]"), DaxError::Evaluation { .. }));
    assert!(matches!(
        f.err("CALCULATE(1, Product[Color] = Sales[Qty])"),
        DaxError::Argument { .. }
    ));
}

#[test]
fn deep_nesting_is_bounded_and_does_not_overflow() {
    let f = fx();
    let nested = |open: &str, inner: &str, close: &str, n: usize| {
        format!("{}{inner}{}", open.repeat(n), close.repeat(n))
    };
    let src = nested(
        "CALCULATE(",
        "SUMX(FILTER(Sales, Sales[Qty] > 0), Sales[Qty])",
        ")",
        120,
    );
    assert_eq!(f.eval_in(&src, &FilterContext::new()), i(15));
    let src = nested("IF(TRUE(), ", "1", ", 0)", 200);
    assert_eq!(f.eval_in(&src, &FilterContext::new()), i(1));
    assert!(matches!(
        f.err(&nested("(", "1", ")", 300)),
        DaxError::Parse { .. }
    ));
    assert!(matches!(
        f.err("SUM(Product[Name])"),
        DaxError::Argument { function, .. } if function == "SUM"
    ));
}

#[test]
fn cancellation_stops_evaluation() {
    let f = fx();
    let flag = Arc::new(AtomicBool::new(true));
    let ctx =
        EvalContext::new(f.engine.snapshot(), Arc::new(f.model.clone())).with_cancel_flag(flag);
    assert!(matches!(
        ctx.evaluate_scalar("SUMX(Sales, Sales[Qty])"),
        Err(DaxError::Cancelled)
    ));
    assert!(matches!(
        ctx.compute_calculated_column("Sales", "Line"),
        Err(DaxError::Cancelled)
    ));
    let filtered = EvalContext::new(f.engine.snapshot(), Arc::new(f.model.clone()))
        .with_filter(FilterContext::new().with(ColumnFilter::equals("Product", "Color", "Red")));
    let tables = filtered
        .evaluate_query("EVALUATE ROW(\"Q\", [Total Qty])")
        .unwrap();
    assert_eq!(tables[0].rows(), vec![vec![i(5)]]);
}

#[test]
fn eval_context_with_row_context_does_context_transition() {
    let f = fx();
    let ctx = EvalContext::new(f.engine.snapshot(), Arc::new(f.model.clone()))
        .with_base_row("Product", 1)
        .unwrap();
    assert_eq!(ctx.evaluate_scalar("Product[Name]").unwrap(), s("Banana"));
    assert_eq!(ctx.evaluate_measure("Total Qty").unwrap(), i(5));
    assert_eq!(ctx.evaluate_scalar("SUM(Sales[Qty])").unwrap(), i(15));
}

#[test]
fn table_results_have_lineage_and_names() {
    let f = fx();
    let t = evaluate_table(
        &f.engine.snapshot(),
        &f.model,
        "ADDCOLUMNS(VALUES(Product[Category]), \"Q\", [Total Qty])",
        &FilterContext::new(),
    )
    .unwrap();
    let names: Vec<String> = t.columns().iter().map(|c| c.qualified_name()).collect();
    assert_eq!(names, ["Product[Category]", "[Q]"]);
    assert_eq!(
        t.rows(),
        vec![
            vec![s("Fruit"), i(10)],
            vec![s("Veg"), i(1)],
            vec![Value::Blank, i(4)],
        ]
    );
    let rb = pbi_dax::to_record_batch(&t);
    assert_eq!(rb.num_columns(), 2);
    assert_eq!(rb.column(1).data_type(), &arrow::datatypes::DataType::Int64);
}

#[test]
fn more_date_functions() {
    fx().check(&date_more_cases());
}

#[test]
fn ranking_and_statistics() {
    fx().check(&stats_cases());
}

#[test]
fn window_functions() {
    fx().check(&window_cases());
}

#[test]
fn text_info_and_table_extras() {
    fx().check(&misc_cases());
}

#[test]
fn visual_calculation_functions() {
    fx().check(&visual_cases());
}

#[test]
fn allselected_and_isinscope_follow_the_data_point_api() {
    let f = fx();
    let outer = FilterContext::new().with(ColumnFilter::equals("Product", "Category", "Fruit"));
    let base = EvalContext::new(f.engine.snapshot(), Arc::new(f.model.clone())).with_filter(outer);
    let red = base
        .clone()
        .with_data_point(&[(ColumnRef::new("Product", "Color"), s("Red"))]);
    let eval = |ctx: &EvalContext, src: &str| ctx.evaluate_scalar(src).unwrap();
    assert_eq!(eval(&red, "[Total Qty]"), i(5));
    assert_eq!(eval(&red, "CALCULATE([Total Qty], ALLSELECTED())"), i(10));
    assert_eq!(
        eval(&red, "CALCULATE([Total Qty], ALLSELECTED(Product[Color]))"),
        i(10)
    );
    assert_eq!(eval(&red, "CALCULATE([Total Qty], ALL())"), i(15));
    assert_eq!(eval(&red, "ISINSCOPE(Product[Color])"), b(true));
    assert_eq!(eval(&red, "ISINSCOPE(Product[Category])"), b(false));
    assert_eq!(
        eval(
            &red,
            "CALCULATE(ISINSCOPE(Product[Color]), ALL(Product[Color]))"
        ),
        b(false)
    );
    assert_eq!(red.evaluate_measure("Red Qty").unwrap(), i(5));
    // Grand total: no group-by values.
    let total = base.clone().with_data_point(&[]);
    assert_eq!(eval(&total, "ISINSCOPE(Product[Color])"), b(false));
    assert_eq!(eval(&total, "CALCULATE([Total Qty], ALLSELECTED())"), i(10));
    // The same with the explicit setters (outer context = no filter).
    let explicit = EvalContext::new(f.engine.snapshot(), Arc::new(f.model.clone()))
        .with_filter(FilterContext::new().with(ColumnFilter::equals("Product", "Color", "Red")))
        .with_allselected(FilterContext::new())
        .with_group_by([ColumnRef::new("Product", "Color")]);
    assert_eq!(
        eval(&explicit, "CALCULATE([Total Qty], ALLSELECTED())"),
        i(15)
    );
    assert_eq!(eval(&explicit, "ISINSCOPE(Product[Color])"), b(true));
    // Queries see the shadow context too.
    let tables = red
        .evaluate_query("EVALUATE ROW(\"T\", CALCULATE([Total Qty], ALLSELECTED()))")
        .unwrap();
    assert_eq!(tables[0].rows(), vec![vec![i(10)]]);
}

#[test]
fn summarizecolumns_rollup_rows_and_scope() {
    let f = fx();
    let q = "EVALUATE SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Product[Category], \"IsTotal\"), \"Q\", [Total Qty], \"InScope\", ISINSCOPE(Product[Category])) ORDER BY [IsTotal], Product[Category]";
    let t = &pbi_dax::evaluate_query(&f.engine.snapshot(), &f.model, q).unwrap()[0];
    let names: Vec<String> = t.columns().iter().map(|c| c.qualified_name()).collect();
    assert_eq!(
        names,
        ["Product[Category]", "[IsTotal]", "[Q]", "[InScope]"]
    );
    assert_eq!(
        t.rows(),
        vec![
            vec![Value::Blank, b(false), i(4), b(true)],
            vec![s("Fruit"), b(false), i(10), b(true)],
            vec![s("Veg"), b(false), i(1), b(true)],
            vec![Value::Blank, b(true), i(15), b(false)],
        ]
    );
    let q = "EVALUATE SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Product[Category], \"C\", Product[Color], \"K\"), TREATAS({\"Veg\"}, Product[Category]), \"Q\", [Total Qty]) ORDER BY [C], [K]";
    let t = &pbi_dax::evaluate_query(&f.engine.snapshot(), &f.model, q).unwrap()[0];
    assert_eq!(
        t.rows(),
        vec![
            vec![s("Veg"), s("Orange"), b(false), b(false), i(1)],
            vec![s("Veg"), Value::Blank, b(false), b(true), i(1)],
            vec![Value::Blank, Value::Blank, b(true), b(true), i(1)],
        ]
    );
}

/// `Orders[OrderDate] → 'Date'[Date]` (active) and `Orders[ShipDate] →
/// 'Date'[Date]` (inactive), a contiguous date table for 2023–2024, and a
/// parent-child `Emp` table.
fn dates_fx() -> Fx {
    let mut dates = Table::new("Date");
    dates.columns = vec![
        Column::data("Date", DataType::DateTime),
        Column::data("Year", DataType::Int64),
        Column::data("Month", DataType::Int64),
    ];
    let mut orders = Table::new("Orders");
    orders.columns = vec![
        Column::data("OrderId", DataType::Int64),
        Column::data("OrderDate", DataType::DateTime),
        Column::data("ShipDate", DataType::DateTime),
        Column::data("Amount", DataType::Int64),
        Column::data("Region", DataType::String),
    ];
    orders.measures = vec![Measure::new("Amt", "SUM(Orders[Amount])")];
    let mut emp = Table::new("Emp");
    emp.columns = vec![
        Column::data("Id", DataType::Int64),
        Column::data("Parent", DataType::Int64),
    ];
    let mut ship = Relationship::new(
        "ship",
        ColumnRef::new("Orders", "ShipDate"),
        ColumnRef::new("Date", "Date"),
    );
    ship.is_active = false;
    let model = Model {
        tables: vec![dates, orders, emp],
        relationships: vec![
            Relationship::new(
                "order",
                ColumnRef::new("Orders", "OrderDate"),
                ColumnRef::new("Date", "Date"),
            ),
            ship,
        ],
        ..Model::default()
    };
    let mut engine = Engine::new(&model);
    let mut days = Vec::new();
    let mut day = chrono::NaiveDate::from_ymd_opt(2023, 1, 1).unwrap();
    while day <= chrono::NaiveDate::from_ymd_opt(2024, 12, 31).unwrap() {
        days.push(day);
        day = day.succ_opt().unwrap();
    }
    use chrono::Datelike;
    let parts: Vec<Option<common::data::DateParts>> = days
        .iter()
        .map(|d| Some((i64::from(d.year()), d.month(), d.day(), 0, 0, 0)))
        .collect();
    let years: Vec<Option<i64>> = days.iter().map(|d| Some(i64::from(d.year()))).collect();
    let months: Vec<Option<i64>> = days.iter().map(|d| Some(i64::from(d.month()))).collect();
    engine
        .load_table(
            "Date",
            batch(vec![
                ("Date", datetimes(&parts)),
                ("Year", ints(&years)),
                ("Month", ints(&months)),
            ]),
        )
        .unwrap();
    let ymd = |y, m, dd| Some((y, m, dd, 0, 0, 0));
    engine
        .load_table(
            "Orders",
            batch(vec![
                ("OrderId", ints(&(1..=8).map(Some).collect::<Vec<_>>())),
                (
                    "OrderDate",
                    datetimes(&[
                        ymd(2023, 1, 10),
                        ymd(2023, 2, 20),
                        ymd(2023, 12, 31),
                        ymd(2024, 1, 15),
                        ymd(2024, 2, 10),
                        ymd(2024, 2, 29),
                        ymd(2024, 3, 31),
                        ymd(2024, 6, 30),
                    ]),
                ),
                (
                    "ShipDate",
                    datetimes(&[
                        ymd(2023, 1, 15),
                        ymd(2023, 3, 2),
                        ymd(2024, 1, 2),
                        ymd(2024, 2, 1),
                        ymd(2024, 2, 12),
                        ymd(2024, 3, 1),
                        ymd(2024, 4, 2),
                        ymd(2024, 7, 1),
                    ]),
                ),
                ("Amount", ints(&[10, 20, 30, 40, 50, 60, 70, 80].map(Some))),
                (
                    "Region",
                    texts(&[
                        "East", "West", "East", "West", "East", "West", "East", "West",
                    ]),
                ),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Emp",
            batch(vec![
                ("Id", ints(&[Some(1), Some(2), Some(3), Some(4)])),
                ("Parent", ints(&[None, Some(1), Some(2), Some(1)])),
            ]),
        )
        .unwrap();
    Fx { engine, model }
}

fn month(y: i64, m: i64) -> FilterContext {
    FilterContext::new()
        .with(ColumnFilter::equals("Date", "Year", y))
        .with(ColumnFilter::equals("Date", "Month", m))
}

/// Evaluated with `'Date'` filtered to February 2024.
fn time_intelligence_cases() -> Vec<(&'static str, Value)> {
    vec![
        ("[Amt]", i(110)),
        ("TOTALYTD([Amt], 'Date'[Date])", i(150)),
        (
            "TOTALYTD([Amt], 'Date'[Date], Orders[Region] = \"West\")",
            i(100),
        ),
        ("TOTALYTD([Amt], 'Date'[Date], \"6/30\")", i(180)),
        ("TOTALQTD([Amt], 'Date'[Date])", i(150)),
        ("TOTALMTD([Amt], 'Date'[Date])", i(110)),
        ("CALCULATE([Amt], DATESYTD('Date'[Date]))", i(150)),
        ("COUNTROWS(DATESYTD('Date'[Date], \"6/30\"))", i(244)),
        ("COUNTROWS(DATESQTD('Date'[Date]))", i(60)),
        ("COUNTROWS(DATESMTD('Date'[Date]))", i(29)),
        ("CALCULATE([Amt], SAMEPERIODLASTYEAR('Date'[Date]))", i(20)),
        ("COUNTROWS(SAMEPERIODLASTYEAR('Date'[Date]))", i(28)),
        ("CALCULATE([Amt], DATEADD('Date'[Date], -1, MONTH))", i(40)),
        ("COUNTROWS(DATEADD('Date'[Date], -1, MONTH))", i(31)),
        ("CALCULATE([Amt], DATEADD('Date'[Date], -1, YEAR))", i(20)),
        ("FIRSTDATE(DATEADD('Date'[Date], 1, DAY))", date(2024, 2, 2)),
        (
            "CALCULATE([Amt], PARALLELPERIOD('Date'[Date], -1, YEAR))",
            i(60),
        ),
        (
            "CALCULATE([Amt], PARALLELPERIOD('Date'[Date], 1, QUARTER))",
            i(80),
        ),
        ("CALCULATE([Amt], PREVIOUSMONTH('Date'[Date]))", i(40)),
        ("CALCULATE([Amt], NEXTMONTH('Date'[Date]))", i(70)),
        ("CALCULATE([Amt], PREVIOUSYEAR('Date'[Date]))", i(60)),
        ("COUNTROWS(NEXTYEAR('Date'[Date]))", Value::Blank),
        ("COUNTROWS(PREVIOUSQUARTER('Date'[Date]))", i(92)),
        ("COUNTROWS(NEXTQUARTER('Date'[Date]))", i(91)),
        ("PREVIOUSDAY('Date'[Date])", date(2024, 1, 31)),
        ("NEXTDAY('Date'[Date])", date(2024, 3, 1)),
        ("STARTOFMONTH('Date'[Date])", date(2024, 2, 1)),
        ("ENDOFMONTH('Date'[Date])", date(2024, 2, 29)),
        ("STARTOFQUARTER('Date'[Date])", date(2024, 1, 1)),
        ("ENDOFQUARTER('Date'[Date])", date(2024, 3, 31)),
        ("STARTOFYEAR('Date'[Date])", date(2024, 1, 1)),
        ("ENDOFYEAR('Date'[Date])", date(2024, 12, 31)),
        ("STARTOFYEAR('Date'[Date], \"6/30\")", date(2023, 7, 1)),
        ("FIRSTDATE('Date'[Date])", date(2024, 2, 1)),
        ("LASTDATE('Date'[Date])", date(2024, 2, 29)),
        (
            "CALCULATE([Amt], DATESBETWEEN('Date'[Date], DATE(2024, 1, 1), DATE(2024, 2, 15)))",
            i(90),
        ),
        (
            "CALCULATE([Amt], DATESBETWEEN('Date'[Date], BLANK(), DATE(2023, 12, 31)))",
            i(60),
        ),
        (
            "CALCULATE([Amt], DATESINPERIOD('Date'[Date], DATE(2024, 3, 31), -3, MONTH))",
            i(220),
        ),
        (
            "COUNTROWS(DATESINPERIOD('Date'[Date], DATE(2024, 1, 1), 1, YEAR))",
            i(366),
        ),
        ("FIRSTNONBLANK('Date'[Date], [Amt])", date(2024, 2, 10)),
        ("LASTNONBLANK('Date'[Date], [Amt])", date(2024, 2, 29)),
        ("FIRSTNONBLANKVALUE('Date'[Date], [Amt])", i(50)),
        ("LASTNONBLANKVALUE('Date'[Date], [Amt])", i(60)),
        ("CLOSINGBALANCEMONTH([Amt], 'Date'[Date])", i(60)),
        ("CLOSINGBALANCEQUARTER([Amt], 'Date'[Date])", i(70)),
        ("CLOSINGBALANCEYEAR([Amt], 'Date'[Date])", Value::Blank),
        ("OPENINGBALANCEMONTH([Amt], 'Date'[Date])", Value::Blank),
        ("OPENINGBALANCEQUARTER([Amt], 'Date'[Date])", i(30)),
        ("OPENINGBALANCEYEAR([Amt], 'Date'[Date])", i(30)),
        (
            "CALCULATE([Amt], USERELATIONSHIP(Orders[ShipDate], 'Date'[Date]))",
            i(90),
        ),
        (
            "CALCULATE([Amt], USERELATIONSHIP('Date'[Date], Orders[ShipDate]))",
            i(90),
        ),
        (
            "CALCULATE([Amt], CROSSFILTER(Orders[OrderDate], 'Date'[Date], None))",
            i(360),
        ),
        (
            "CALCULATE([Amt], CROSSFILTER(Orders[OrderDate], 'Date'[Date], OneWay_LeftFiltersRight))",
            i(360),
        ),
        (
            "CALCULATE(COUNTROWS('Date'), Orders[Region] = \"East\", CROSSFILTER(Orders[OrderDate], 'Date'[Date], Both), ALL('Date'))",
            i(4),
        ),
        (
            "CALCULATE(COUNTROWS('Date'), Orders[Region] = \"East\", CROSSFILTER(Orders[OrderDate], 'Date'[Date], OneWay), ALL('Date'))",
            i(731),
        ),
        (
            "CALCULATE(TOTALYTD([Amt], 'Date'[Date]), USERELATIONSHIP(Orders[ShipDate], 'Date'[Date]))",
            i(120),
        ),
        (
            "CONCATENATEX(Emp, PATH(Emp[Id], Emp[Parent]), \";\", Emp[Id], ASC)",
            s("1;1|2;1|2|3;1|4"),
        ),
    ]
}

#[test]
fn time_intelligence_and_relationship_modifiers() {
    let f = dates_fx();
    f.check_in(&time_intelligence_cases(), &month(2024, 2));
    f.check_in(
        &[
            ("OPENINGBALANCEMONTH([Amt], 'Date'[Date])", i(60)),
            ("CALCULATE([Amt], PREVIOUSMONTH('Date'[Date]))", i(110)),
        ],
        &month(2024, 3),
    );
    // Without a date filter: the whole visible range.
    f.check(&[
        ("TOTALYTD([Amt], 'Date'[Date])", i(300)),
        ("COUNTROWS(DATESYTD('Date'[Date]))", i(366)),
        ("CALCULATE([Amt], SAMEPERIODLASTYEAR('Date'[Date]))", i(60)),
    ]);
    // In a row context the dates come from context transition.
    f.check(&[(
        "MINX(VALUES('Date'[Year]), TOTALYTD([Amt], 'Date'[Date]))",
        i(60),
    )]);
}

#[test]
fn functions_md_lists_every_supported_function() {
    let doc =
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/FUNCTIONS.md")).unwrap();
    let missing: Vec<&str> = supported_functions()
        .into_iter()
        .filter(|f| !doc.contains(&format!("| `{f}` |")))
        .collect();
    assert!(missing.is_empty(), "missing from FUNCTIONS.md: {missing:?}");
}

/// Every registered function appears in at least one test expression.
#[test]
fn every_supported_function_is_tested() {
    let mut sources: Vec<&str> = Vec::new();
    for cases in [
        logical_cases(),
        math_cases(),
        text_cases(),
        date_cases(),
        aggregate_cases(),
        context_cases(),
        table_cases(),
        date_more_cases(),
        stats_cases(),
        window_cases(),
        misc_cases(),
        visual_cases(),
        time_intelligence_cases(),
        library_cases(),
    ] {
        sources.extend(cases.into_iter().map(|(src, _)| src));
    }
    sources.extend(library_approx_cases().into_iter().map(|(src, _)| src));
    let corpus = sources.join("\n").to_uppercase();
    // Covered in dedicated tests above, or in `tests/model_features.rs`
    // (calculation groups need a calculation-group table).
    let elsewhere = [
        "REMOVEFILTERS",
        "ALLEXCEPT",
        "SELECTEDMEASURE",
        "SELECTEDMEASURENAME",
        "SELECTEDMEASUREFORMATSTRING",
        "ISSELECTEDMEASURE",
    ];
    let missing: Vec<&str> = supported_functions()
        .into_iter()
        .filter(|f| !elsewhere.contains(f) && !corpus.contains(&format!("{f}(")))
        .collect();
    assert!(missing.is_empty(), "functions without a test: {missing:?}");
}
