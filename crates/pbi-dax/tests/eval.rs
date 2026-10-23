#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Evaluator tests on the example model (`AcmeOutfitters.SemanticModel`,
//! committed under `reference/examples/acme-outfitters`) with small
//! synthetic data: calculated columns, the calculated `Date` and
//! `Products Never Sold` tables, measures with and without a region filter,
//! relationships in both directions, context transition, time
//! intelligence, queries and dependencies.

mod common;

use std::collections::HashSet;

use common::data::{batch, bools, dates, decimals, floats, ints, reference_root, strings, texts};
use pbi_dax::{
    DaxError, Dependency, compute_calculated_column, compute_calculated_table, dependencies_in,
    evaluate_measure, evaluate_query, evaluate_scalar, evaluate_table, to_record_batch,
    transitive_dependencies,
};
use pbi_engine::{ColumnFilter, Engine, FilterContext, Value};
use pbi_model::Model;
use pretty_assertions::assert_eq;

struct Fixture {
    engine: Engine,
    model: Model,
}

/// The example's TMDL model, or `None` (after printing `SKIP`).
fn example_model() -> Option<Model> {
    let dir = reference_root().join("examples/acme-outfitters/AcmeOutfitters.SemanticModel");
    if !dir.is_dir() {
        println!("SKIP: {} missing", dir.display());
        return None;
    }
    Some(pbi_tmdl::load_model_from_dir(&dir).unwrap().model)
}

/// Computes the calculated tables and columns the tests use, in dependency
/// order, like a refresh does.
fn compute_calculated(engine: &mut Engine, model: &Model) {
    let date = compute_calculated_table(&engine.snapshot(), model, "Date").unwrap();
    engine.load_table("Date", date).unwrap();
    for (table, column) in [
        ("Date", "Day Type"),
        ("Orders", "Line Cost"),
        ("Orders", "Is Returned"),
        ("Orders", "Order Size"),
        ("Orders", "Days To Ship"),
        ("Customers", "Lifetime Sales"),
        ("Customers", "Has Orders"),
        ("Customers", "Value Tier"),
        ("Products", "Units Sold (All Time)"),
        ("Products", "Price Band"),
        ("Stores", "Store Label"),
    ] {
        let array = compute_calculated_column(&engine.snapshot(), model, table, column)
            .unwrap_or_else(|e| panic!("{table}[{column}]: {e}"));
        engine.add_column(table, column, array).unwrap();
    }
    let never_sold =
        compute_calculated_table(&engine.snapshot(), model, "Products Never Sold").unwrap();
    engine
        .load_table("Products Never Sold", never_sold)
        .unwrap();
}

/// Three stores (S1 and S3 in the West), five products (P4 and P5 never
/// sold), four customers (C4 without orders) and seven orders: O5 has an
/// unknown customer (C9), O6 an unknown product (P9), O7 an unknown store
/// (S9); O4 is the only 2024 order. Two returns, three web sessions.
fn fixture() -> Option<Fixture> {
    let model = example_model()?;
    let mut engine = Engine::new(&model);
    engine
        .load_table(
            "Stores",
            batch(vec![
                ("Store ID", texts(&["S1", "S2", "S3"])),
                (
                    "Store Name",
                    texts(&["Seattle Flagship", "Boston Back Bay", "Web Store"]),
                ),
                ("Region", texts(&["West", "East", "West"])),
                ("Channel", texts(&["Retail", "Retail", "Online"])),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Products",
            batch(vec![
                ("Product ID", texts(&["P1", "P2", "P3", "P4", "P5"])),
                (
                    "Product Name",
                    texts(&[
                        "Juniper Tent",
                        "Alder Pack",
                        "Cedar Stove",
                        "Birch Poles",
                        "Spruce Lantern",
                    ]),
                ),
                (
                    "Category",
                    texts(&["Camping", "Hiking", "Camping", "Hiking", "Camping"]),
                ),
                (
                    "List Price",
                    decimals(&[Some(200.0), Some(100.0), Some(40.0), Some(30.0), Some(25.0)]),
                ),
                (
                    "Unit Cost",
                    decimals(&[Some(120.0), Some(50.0), Some(25.0), Some(12.0), Some(10.0)]),
                ),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Customers",
            batch(vec![
                ("Customer ID", texts(&["C1", "C2", "C3", "C4"])),
                (
                    "Segment",
                    texts(&["Consumer", "Corporate", "Consumer", "Consumer"]),
                ),
                (
                    "Loyalty Tier",
                    texts(&["Gold", "Silver", "Bronze", "Unassigned"]),
                ),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Orders",
            batch(vec![
                (
                    "Order ID",
                    texts(&["O1", "O2", "O3", "O4", "O5", "O6", "O7"]),
                ),
                (
                    "Order Date",
                    dates(&[
                        Some((2025, 1, 10)),
                        Some((2025, 2, 5)),
                        Some((2025, 3, 15)),
                        Some((2024, 3, 20)),
                        Some((2025, 4, 1)),
                        Some((2025, 4, 2)),
                        Some((2025, 5, 5)),
                    ]),
                ),
                (
                    "Ship Date",
                    dates(&[
                        Some((2025, 1, 12)),
                        Some((2025, 2, 6)),
                        None,
                        Some((2024, 3, 25)),
                        Some((2025, 4, 3)),
                        None,
                        Some((2025, 5, 7)),
                    ]),
                ),
                (
                    "Customer ID",
                    texts(&["C1", "C1", "C2", "C3", "C9", "C2", "C3"]),
                ),
                (
                    "Store ID",
                    texts(&["S1", "S2", "S3", "S1", "S2", "S1", "S9"]),
                ),
                (
                    "Product ID",
                    texts(&["P1", "P2", "P3", "P2", "P1", "P9", "P3"]),
                ),
                (
                    "Quantity",
                    ints(&[
                        Some(6),
                        Some(1),
                        Some(3),
                        Some(2),
                        Some(1),
                        Some(1),
                        Some(2),
                    ]),
                ),
                (
                    "Unit Price",
                    floats(&[
                        Some(200.0),
                        Some(100.0),
                        Some(40.0),
                        Some(100.0),
                        Some(200.0),
                        Some(50.0),
                        Some(40.0),
                    ]),
                ),
                (
                    "Discount",
                    floats(&[
                        Some(0.0),
                        Some(0.0),
                        Some(0.0),
                        Some(0.1),
                        Some(0.0),
                        Some(0.0),
                        Some(0.0),
                    ]),
                ),
                (
                    "Status",
                    texts(&[
                        "Completed",
                        "Completed",
                        "Cancelled",
                        "Completed",
                        "Completed",
                        "Pending",
                        "Completed",
                    ]),
                ),
                (
                    "Line Amount",
                    floats(&[
                        Some(1200.0),
                        Some(100.0),
                        Some(120.0),
                        Some(180.0),
                        Some(200.0),
                        Some(50.0),
                        Some(80.0),
                    ]),
                ),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Returns",
            batch(vec![
                ("Return ID", texts(&["R1", "R2"])),
                ("Order ID", texts(&["O2", "O4"])),
                ("Reason", texts(&["Wrong Size", "Changed Mind"])),
                ("Refund Amount", decimals(&[Some(100.0), Some(90.0)])),
            ]),
        )
        .unwrap();
    engine
        .load_table(
            "Web Sessions",
            batch(vec![
                ("Session ID", texts(&["W1", "W2", "W3"])),
                ("Device", texts(&["Mobile", "Desktop", "Mobile"])),
                ("Pages Viewed", ints(&[Some(4), Some(6), Some(2)])),
                ("Converted", bools(&[Some(true), Some(false), Some(false)])),
            ]),
        )
        .unwrap();
    compute_calculated(&mut engine, &model);
    Some(Fixture { engine, model })
}

fn west() -> FilterContext {
    FilterContext::new().with(ColumnFilter::equals("Stores", "Region", "West"))
}

impl Fixture {
    fn measure(&self, name: &str, filter: &FilterContext) -> Value {
        evaluate_measure(&self.engine.snapshot(), &self.model, name, filter)
            .unwrap_or_else(|e| panic!("[{name}]: {e}"))
    }

    fn scalar_in(&self, src: &str, filter: &FilterContext) -> Value {
        evaluate_scalar(&self.engine.snapshot(), &self.model, src, filter)
            .unwrap_or_else(|e| panic!("{src}: {e}"))
    }

    fn scalar(&self, src: &str) -> Value {
        self.scalar_in(src, &FilterContext::new())
    }

    fn column(&self, table: &str, column: &str) -> Vec<Value> {
        let snap = self.engine.snapshot();
        let n = snap.row_count(table).unwrap();
        (0..n)
            .map(|r| snap.value_at(table, column, r).unwrap())
            .collect()
    }
}

fn s(v: &str) -> Value {
    Value::from(v)
}

fn d(v: f64) -> Value {
    Value::Double(v)
}

#[test]
fn calculated_columns() {
    let Some(f) = fixture() else { return };
    // IF(ISEMPTY(RELATEDTABLE(Orders)), …): C4 has no orders.
    assert_eq!(
        f.column("Customers", "Has Orders"),
        vec![s("Yes"), s("Yes"), s("Yes"), s("No")]
    );
    // Context transition: every order of the customer.
    assert_eq!(
        f.column("Customers", "Lifetime Sales"),
        vec![d(1300.0), d(170.0), d(260.0), Value::Blank]
    );
    // SWITCH(TRUE(), …) over another calculated column.
    assert_eq!(
        f.column("Customers", "Value Tier"),
        vec![s("Medium"), s("Low"), s("Low"), s("None")]
    );
    // SUMX(RELATEDTABLE(Orders), …): BLANK for products never sold.
    assert_eq!(
        f.column("Products", "Units Sold (All Time)"),
        vec![
            Value::Integer(7),
            Value::Integer(3),
            Value::Integer(5),
            Value::Blank,
            Value::Blank
        ]
    );
    assert_eq!(
        f.column("Products", "Price Band"),
        vec![
            s("$150 and up"),
            s("$50 to $149"),
            s("Under $50"),
            s("Under $50"),
            s("Under $50")
        ]
    );
    // RELATED to the one side: BLANK for the unknown product P9.
    assert_eq!(
        f.column("Orders", "Line Cost"),
        vec![
            d(720.0),
            d(50.0),
            d(75.0),
            d(100.0),
            d(120.0),
            Value::Blank,
            d(50.0)
        ]
    );
    // One-to-one relationship to Returns.
    assert_eq!(
        f.column("Orders", "Is Returned"),
        ["No", "Yes", "No", "Yes", "No", "No", "No"].map(s).to_vec()
    );
    assert_eq!(
        f.column("Orders", "Order Size")[..2],
        [s("Large"), s("Small")]
    );
    assert_eq!(
        f.column("Orders", "Days To Ship"),
        vec![
            Value::Integer(2),
            Value::Integer(1),
            Value::Blank,
            Value::Integer(5),
            Value::Integer(2),
            Value::Blank,
            Value::Integer(2)
        ]
    );
    assert_eq!(
        f.column("Stores", "Store Label")[0],
        s("Seattle Flagship (West)")
    );
    let snap = f.engine.snapshot();
    let ty = snap
        .table("Products")
        .unwrap()
        .column_type("Units Sold (All Time)")
        .unwrap()
        .clone();
    assert_eq!(ty, arrow::datatypes::DataType::Int64);
}

#[test]
fn calculated_tables() {
    let Some(f) = fixture() else { return };
    assert_eq!(
        f.column("Products Never Sold", "Product ID"),
        vec![s("P4"), s("P5")]
    );
    assert_eq!(
        f.column("Products Never Sold", "Product"),
        vec![s("Birch Poles"), s("Spruce Lantern")]
    );
    // CALENDAR + ADDCOLUMNS: 2024-01-01 … 2025-12-31.
    let dates = f.column("Date", "Date");
    assert_eq!(dates.len(), 731);
    assert_eq!(dates[0], Value::date(2024, 1, 1).unwrap());
    assert_eq!(dates[730], Value::date(2025, 12, 31).unwrap());
    assert_eq!(f.column("Date", "Year Month")[59], s("2024-02"));
    assert_eq!(f.column("Date", "Quarter")[730], s("Q4"));
    // 2024-01-06 was a Saturday.
    assert_eq!(f.column("Date", "Day Type")[5], s("Weekend"));
}

#[test]
fn measures_unfiltered() {
    let Some(f) = fixture() else { return };
    let none = FilterContext::new();
    assert_eq!(f.measure("Total Sales", &none), d(1930.0));
    assert_eq!(f.measure("Total Cost", &none), d(1115.0));
    assert_eq!(f.measure("Gross Margin", &none), d(815.0));
    assert_eq!(f.measure("Margin %", &none), d(815.0 / 1930.0));
    assert_eq!(f.measure("Completed Sales", &none), d(1760.0));
    assert_eq!(f.measure("Order Count", &none), Value::Integer(7));
    assert_eq!(f.measure("Units Sold", &none), Value::Integer(16));
    // Unknown customer ids count as active customers, not as customers.
    assert_eq!(f.measure("Active Customers", &none), Value::Integer(4));
    assert_eq!(f.measure("Customer Count", &none), Value::Integer(4));
    assert_eq!(
        f.measure("Unknown Product Orders", &none),
        Value::Integer(1)
    );
    assert_eq!(
        f.measure("Products Never Sold Count", &none),
        Value::Integer(2)
    );
    assert_eq!(f.measure("Returns Count", &none), Value::Integer(2));
    assert_eq!(f.measure("Refunds", &none), Value::Decimal(1_900_000));
    assert_eq!(f.measure("Return Rate", &none), d(2.0 / 7.0));
    assert_eq!(f.measure("Sessions", &none), Value::Integer(3));
    assert_eq!(f.measure("Conversion Rate", &none), d(1.0 / 3.0));
    assert_eq!(f.measure("Avg Pages per Session", &none), d(4.0));
    assert_eq!(
        f.measure("Last Order Date", &none),
        Value::date(2025, 5, 5).unwrap()
    );
}

#[test]
fn measures_under_a_region_filter() {
    let Some(f) = fixture() else { return };
    let west = west();
    // O1, O3, O4 and O6; O7's unknown store follows the blank row, which
    // fails the filter.
    assert_eq!(f.measure("Order Count", &west), Value::Integer(4));
    assert_eq!(f.measure("Total Sales", &west), d(1550.0));
    assert_eq!(f.measure("Active Customers", &west), Value::Integer(3));
    // Orders → Customers filters both ways.
    assert_eq!(f.measure("Customer Count", &west), Value::Integer(3));
    assert_eq!(
        f.measure("Unknown Product Orders", &west),
        Value::Integer(1)
    );
    // The one-to-one relationship carries the filter to Returns.
    assert_eq!(f.measure("Returns Count", &west), Value::Integer(1));
    // ALL(Stores) brings back every store and the blank row.
    assert_eq!(f.measure("Sales All Stores", &west), d(1930.0));
    assert_eq!(f.measure("Store Share", &west), d(1550.0 / 1930.0));
    // No relationship path to the calculated table or to Web Sessions.
    assert_eq!(
        f.measure("Products Never Sold Count", &west),
        Value::Integer(2)
    );
    assert_eq!(f.measure("Sessions", &west), Value::Integer(3));
    assert_eq!(
        f.measure("'Orders'[Last Order Date]", &west),
        Value::date(2025, 4, 2).unwrap()
    );
}

#[test]
fn bidirectional_relationship_filters_customers_from_orders() {
    let Some(f) = fixture() else { return };
    let cancelled =
        FilterContext::new().with(ColumnFilter::equals("Orders", "Status", "Cancelled"));
    // O3 only: its customer C2.
    assert_eq!(f.measure("Order Count", &cancelled), Value::Integer(1));
    assert_eq!(f.measure("Customer Count", &cancelled), Value::Integer(1));
    // Orders → Products is single-direction.
    assert_eq!(
        f.scalar_in("COUNTROWS(Products)", &cancelled),
        Value::Integer(5)
    );
}

#[test]
fn time_intelligence_over_the_calculated_date_table() {
    let Some(f) = fixture() else { return };
    let month = |m: &str| FilterContext::new().with(ColumnFilter::equals("Date", "Year Month", m));
    let march = month("2025-03");
    assert_eq!(f.measure("Total Sales", &march), d(120.0));
    assert_eq!(f.measure("Sales YTD", &march), d(1420.0));
    assert_eq!(f.measure("Sales PY", &march), d(180.0));
    assert_eq!(f.measure("Sales YoY %", &march), d((120.0 - 180.0) / 180.0));
    assert_eq!(f.measure("Sales Prev Month", &march), d(100.0));
    // USERELATIONSHIP(Orders[Ship Date], …): O1 shipped in January 2025.
    assert_eq!(f.measure("Shipped Sales", &month("2025-01")), d(1200.0));
    assert_eq!(f.measure("Shipped Sales", &march), Value::Blank);
    let year = FilterContext::new().with(ColumnFilter::equals("Date", "Year", 2025));
    assert_eq!(f.measure("Total Sales", &year), d(1750.0));
}

#[test]
fn context_transition_in_iterators_and_calculate() {
    let Some(f) = fixture() else { return };
    // [Order Count] per customer (context transition), summed: C1, C2 and
    // C3 have two orders each; O5's unknown customer belongs to no row.
    assert_eq!(
        f.scalar("SUMX(Customers, [Order Count])"),
        Value::Integer(6)
    );
    // Without context transition every row sees all orders.
    assert_eq!(
        f.scalar("SUMX(Customers, COUNTROWS(Orders))"),
        Value::Integer(28)
    );
    assert_eq!(
        f.scalar("SUMX(Customers, CALCULATE(COUNTROWS(Orders)))"),
        Value::Integer(6)
    );
    assert_eq!(
        f.scalar("COUNTROWS(FILTER(Customers, [Total Sales] > 200))"),
        Value::Integer(2)
    );
    // CALCULATE with a table filter (expanded table of Orders).
    assert_eq!(
        f.scalar("CALCULATE([Customer Count], FILTER(Orders, Orders[Status] = \"Completed\"))"),
        Value::Integer(2)
    );
    // ALL removes the outer filter; KEEPFILTERS intersects.
    let consumer =
        FilterContext::new().with(ColumnFilter::equals("Customers", "Segment", "Consumer"));
    let snap = f.engine.snapshot();
    let eval = |src: &str| evaluate_scalar(&snap, &f.model, src, &consumer).unwrap();
    assert_eq!(eval("[Customer Count]"), Value::Integer(3));
    assert_eq!(
        eval("CALCULATE([Customer Count], ALL(Customers))"),
        Value::Integer(4)
    );
    assert_eq!(
        eval("CALCULATE([Customer Count], Customers[Segment] = \"Corporate\")"),
        Value::Integer(1)
    );
    assert_eq!(
        eval("CALCULATE([Customer Count], KEEPFILTERS(Customers[Segment] = \"Corporate\"))"),
        Value::Blank
    );
    assert_eq!(
        eval("CALCULATE([Customer Count], REMOVEFILTERS(Customers[Segment]))"),
        Value::Integer(4)
    );
    assert_eq!(
        eval("DIVIDE([Customer Count], CALCULATE([Customer Count], ALL(Customers)))"),
        d(0.75)
    );
}

#[test]
fn values_and_blank_row() {
    let Some(f) = fixture() else { return };
    // O5's unknown customer creates a blank row in Customers: VALUES/ALL
    // see it, table references and DISTINCT do not.
    assert_eq!(
        f.scalar("COUNTROWS(VALUES(Customers[Loyalty Tier]))"),
        Value::Integer(5)
    );
    assert_eq!(
        f.scalar("COUNTROWS(DISTINCT(Customers[Loyalty Tier]))"),
        Value::Integer(4)
    );
    assert_eq!(f.scalar("COUNTROWS(Customers)"), Value::Integer(4));
    assert_eq!(f.scalar("COUNTROWS(ALL(Customers))"), Value::Integer(5));
    assert_eq!(
        f.scalar("COUNTROWS(ALLNOBLANKROW(Customers))"),
        Value::Integer(4)
    );
    assert_eq!(
        f.scalar("DISTINCTCOUNT(Customers[Loyalty Tier])"),
        Value::Integer(5)
    );
}

#[test]
fn queries_with_define_and_order_by() {
    let Some(f) = fixture() else { return };
    let q = r#"
        DEFINE
            VAR _min = 2
            MEASURE Customers[Orders per Customer] = DIVIDE([Order Count], [Customer Count])
            TABLE Frequent = FILTER(Products, Products[Units Sold (All Time)] >= _min)
        EVALUATE
            SUMMARIZECOLUMNS(
                Customers[Segment],
                "Customers", [Customer Count],
                "Ratio", [Orders per Customer]
            )
        ORDER BY [Customers] DESC, Customers[Segment]
        EVALUATE SELECTCOLUMNS(Frequent, "Product", Frequent[Product Name])
        ORDER BY [Product]
        EVALUATE { _min }
    "#;
    let results = evaluate_query(&f.engine.snapshot(), &f.model, q).unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(
        results[0].rows(),
        vec![
            vec![s("Consumer"), Value::Integer(3), d(4.0 / 3.0)],
            vec![s("Corporate"), Value::Integer(1), d(2.0)],
        ]
    );
    let names: Vec<String> = results[0]
        .columns()
        .iter()
        .map(|c| c.qualified_name())
        .collect();
    assert_eq!(names, ["Customers[Segment]", "[Customers]", "[Ratio]"]);
    assert_eq!(
        results[1].rows(),
        vec![
            vec![s("Alder Pack")],
            vec![s("Cedar Stove")],
            vec![s("Juniper Tent")]
        ]
    );
    assert_eq!(results[2].rows(), vec![vec![Value::Integer(2)]]);
    let rb = to_record_batch(&results[0]);
    assert_eq!(rb.num_rows(), 2);
    assert_eq!(rb.schema().field(1).name(), "[Customers]");
}

#[test]
fn table_expressions_keep_lineage_for_filters() {
    let Some(f) = fixture() else { return };
    let snap = f.engine.snapshot();
    let t = evaluate_table(
        &snap,
        &f.model,
        "SUMMARIZE(Orders, Orders[Customer ID], \"Orders\", COUNTROWS(Orders))",
        &FilterContext::new(),
    )
    .unwrap();
    assert_eq!(
        t.rows(),
        vec![
            vec![s("C1"), Value::Integer(2)],
            vec![s("C2"), Value::Integer(2)],
            vec![s("C3"), Value::Integer(2)],
            vec![s("C9"), Value::Integer(1)],
        ]
    );
    // A FILTER over a summarized table applied as a CALCULATE filter.
    assert_eq!(
        f.scalar(
            "CALCULATE(COUNTROWS(Customers), FILTER(VALUES(Orders[Customer ID]), Orders[Customer ID] = \"C2\"))"
        ),
        Value::Integer(1)
    );
    assert_eq!(
        f.scalar("CALCULATE([Order Count], TREATAS({\"C1\"}, Customers[Customer ID]))"),
        Value::Integer(2)
    );
}

#[test]
fn dependencies_resolve_against_the_model() {
    let Some(f) = fixture() else { return };
    let customers = f.model.table("Customers").unwrap();
    let has_orders = customers
        .column("Has Orders")
        .unwrap()
        .expression()
        .unwrap();
    assert_eq!(
        dependencies_in(&f.model, Some("Customers"), has_orders).unwrap(),
        vec![Dependency::Table("Orders".into())]
    );
    let never_sold = f
        .model
        .table("Products Never Sold")
        .unwrap()
        .calculated_expression()
        .unwrap();
    let deps = dependencies_in(&f.model, None, never_sold).unwrap();
    assert!(deps.contains(&Dependency::Table("Orders".into())));
    assert!(deps.contains(&Dependency::Column {
        table: "Products".into(),
        column: "Product ID".into()
    }));
    let deps = transitive_dependencies(&f.model, None, "[Margin %]").unwrap();
    assert!(deps.contains(&Dependency::Measure("Gross Margin".into())));
    assert!(deps.contains(&Dependency::Measure("Total Cost".into())));
    assert!(deps.contains(&Dependency::Column {
        table: "Orders".into(),
        column: "Line Cost".into()
    }));
}

#[test]
fn errors_are_reported_not_panics() {
    let Some(f) = fixture() else { return };
    let snap = f.engine.snapshot();
    let none = FilterContext::new();
    let err = |src: &str| evaluate_scalar(&snap, &f.model, src, &none).unwrap_err();
    assert!(matches!(
        err("TOJSON({1})"),
        DaxError::UnsupportedFunction { name } if name == "TOJSON"
    ));
    // Outside a calculation item there is no selected measure.
    assert!(matches!(
        err("SELECTEDMEASURE()"),
        DaxError::Evaluation { .. }
    ));
    assert!(matches!(err("[Nope]"), DaxError::UnknownMeasure { .. }));
    assert!(matches!(
        err("COUNTROWS(Nope)"),
        DaxError::UnknownTable { .. }
    ));
    assert!(matches!(
        err("SUM(Orders[Nope])"),
        DaxError::UnknownColumn { .. }
    ));
    assert!(matches!(err("1 +"), DaxError::Parse { line: 1, .. }));
    let msg = err("Customers[Segment]").to_string();
    assert!(msg.contains("single value"), "{msg}");
    assert!(matches!(
        evaluate_measure(&snap, &f.model, "Missing", &none),
        Err(DaxError::UnknownMeasure { .. })
    ));
}

struct GeneratedOrder {
    customer: String,
    product: String,
    store: usize,
    quantity: i64,
    amount: f64,
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn customers_of<'a>(rows: &[&'a GeneratedOrder]) -> HashSet<&'a str> {
    rows.iter().map(|o| o.customer.as_str()).collect()
}

/// Example-sized data (3,906 orders, 400 customers, 120 products, 15
/// stores) with unknown customer and product ids, customers without orders
/// and six products never sold; the expected values are computed here in
/// plain Rust.
#[test]
fn example_sizes_match_values_computed_independently() {
    let Some(model) = example_model() else { return };
    let started = std::time::Instant::now();
    let regions = ["West", "East", "Central"];
    let store_ids: Vec<String> = (0..15).map(|i| format!("S{i:02}")).collect();
    let store_regions: Vec<&str> = (0..15).map(|i| regions[i % 3]).collect();
    let product_ids: Vec<String> = (0..120).map(|i| format!("P{i:03}")).collect();
    let product_names: Vec<String> = (0..120).map(|i| format!("Product {i}")).collect();
    let customer_ids: Vec<String> = (0..400).map(|i| format!("C{i:03}")).collect();
    let orders: Vec<GeneratedOrder> = (0..3906usize)
        .map(|i| {
            let quantity = 1 + (i % 3) as i64;
            GeneratedOrder {
                customer: if i % 651 == 0 {
                    format!("X{i}")
                } else {
                    customer_ids[(i * 7) % 390].clone()
                },
                product: if i % 977 == 1 {
                    "P999".to_owned()
                } else {
                    product_ids[i % 114].clone()
                },
                store: i % 15,
                quantity,
                amount: quantity as f64 * (10 + i % 114) as f64,
            }
        })
        .collect();

    let mut engine = Engine::new(&model);
    let order_customers: Vec<Option<&str>> =
        orders.iter().map(|o| Some(o.customer.as_str())).collect();
    let order_products: Vec<Option<&str>> =
        orders.iter().map(|o| Some(o.product.as_str())).collect();
    let order_stores: Vec<Option<&str>> = orders
        .iter()
        .map(|o| Some(store_ids[o.store].as_str()))
        .collect();
    engine
        .load_tables([
            (
                "Stores".to_owned(),
                batch(vec![
                    ("Store ID", texts(&refs(&store_ids))),
                    ("Region", texts(&store_regions)),
                ]),
            ),
            (
                "Products".to_owned(),
                batch(vec![
                    ("Product ID", texts(&refs(&product_ids))),
                    ("Product Name", texts(&refs(&product_names))),
                    ("Category", texts(&["Camping"; 120])),
                    ("List Price", decimals(&[Some(10.0); 120])),
                ]),
            ),
            (
                "Customers".to_owned(),
                batch(vec![("Customer ID", texts(&refs(&customer_ids)))]),
            ),
            (
                "Orders".to_owned(),
                batch(vec![
                    ("Customer ID", strings(&order_customers)),
                    ("Product ID", strings(&order_products)),
                    ("Store ID", strings(&order_stores)),
                    (
                        "Quantity",
                        ints(&orders.iter().map(|o| Some(o.quantity)).collect::<Vec<_>>()),
                    ),
                    (
                        "Line Amount",
                        floats(&orders.iter().map(|o| Some(o.amount)).collect::<Vec<_>>()),
                    ),
                ]),
            ),
        ])
        .unwrap();
    let has_orders =
        compute_calculated_column(&engine.snapshot(), &model, "Customers", "Has Orders").unwrap();
    engine
        .add_column("Customers", "Has Orders", has_orders)
        .unwrap();
    let table =
        compute_calculated_table(&engine.snapshot(), &model, "Products Never Sold").unwrap();
    engine.load_table("Products Never Sold", table).unwrap();

    // Expected values, without the engine.
    let customers: HashSet<&str> = customer_ids.iter().map(String::as_str).collect();
    let sold: HashSet<&str> = orders.iter().map(|o| o.product.as_str()).collect();
    let never_sold = product_ids
        .iter()
        .filter(|p| !sold.contains(p.as_str()))
        .count();
    let known = |ids: &HashSet<&str>| ids.iter().filter(|c| customers.contains(*c)).count();
    let total = |rows: &[&GeneratedOrder]| rows.iter().map(|o| o.amount).sum::<f64>();
    let all: Vec<&GeneratedOrder> = orders.iter().collect();
    let in_west: Vec<&GeneratedOrder> = orders
        .iter()
        .filter(|o| store_regions[o.store] == "West")
        .collect();
    let int = |n: usize| Value::Integer(i64::try_from(n).unwrap());

    let snap = engine.snapshot();
    let m = |name: &str, f: &FilterContext| evaluate_measure(&snap, &model, name, f).unwrap();
    let none = FilterContext::new();
    assert_eq!(never_sold, 6);
    assert_eq!(m("Order Count", &none), int(orders.len()));
    assert_eq!(
        m("Units Sold", &none),
        Value::Integer(orders.iter().map(|o| o.quantity).sum())
    );
    assert_eq!(m("Total Sales", &none), d(total(&all)));
    assert_eq!(m("Active Customers", &none), int(customers_of(&all).len()));
    assert_eq!(m("Customer Count", &none), int(customers.len()));
    assert_eq!(
        m("Unknown Product Orders", &none),
        int(orders.iter().filter(|o| o.product == "P999").count())
    );
    assert_eq!(m("Products Never Sold Count", &none), int(never_sold));
    let west = west();
    assert_eq!(m("Order Count", &west), int(in_west.len()));
    assert_eq!(m("Total Sales", &west), d(total(&in_west)));
    assert_eq!(
        m("Active Customers", &west),
        int(customers_of(&in_west).len())
    );
    assert_eq!(
        m("Customer Count", &west),
        int(known(&customers_of(&in_west)))
    );
    assert_eq!(m("Products Never Sold Count", &west), int(never_sold));
    assert_eq!(
        evaluate_scalar(
            &snap,
            &model,
            "COUNTROWS(FILTER(Customers, Customers[Has Orders] = \"No\"))",
            &none
        )
        .unwrap(),
        int(customers.len() - known(&customers_of(&all)))
    );
    assert_eq!(customers.len() - known(&customers_of(&all)), 10);
    eprintln!("example-sized data evaluated in {:?}", started.elapsed());
}

/// Every DAX sample in `tests/data` evaluates or fails with a regular
/// error (never a panic / internal error).
#[test]
fn sample_expressions_never_fail_internally() {
    let Some(f) = fixture() else { return };
    let snap = f.engine.snapshot();
    let none = FilterContext::new();
    let internal = |e: &DaxError| e.to_string().contains("internal error");
    for sub in ["", "extra"] {
        for (path, src) in common::data_files(sub) {
            if let Err(e) = evaluate_scalar(&snap, &f.model, &src, &none) {
                assert!(!internal(&e), "{}: {e}", path.display());
                if let Err(e) = evaluate_table(&snap, &f.model, &src, &none) {
                    assert!(!internal(&e), "{}: {e}", path.display());
                }
            }
        }
    }
    for (path, src) in common::data_files("queries") {
        if let Err(e) = evaluate_query(&snap, &f.model, &src) {
            assert!(!internal(&e), "{}: {e}", path.display());
        }
    }
    // The model's own expressions all evaluate: measures as scalars, the
    // calculated table as a table.
    for (path, src) in common::data_files("") {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        if name.contains("_col_") {
            continue;
        }
        if name.starts_with("table_") {
            evaluate_table(&snap, &f.model, &src, &none).unwrap_or_else(|e| panic!("{name}: {e}"));
        } else {
            evaluate_scalar(&snap, &f.model, &src, &none).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
    }
}
