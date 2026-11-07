//! Performance scenarios of `pbi-dax` (`cargo xtask perf`; budgets in `perf/budgets.toml`,
//! docs in `docs/perf.md`):
//!
//! - P04: DAX measures over a synthetic 1 000 000-row fact table with a 1 000-row product
//!   dimension. One iteration evaluates every measure of [`MEASURES`] once in an empty filter
//!   context: plain aggregations, `CALCULATE` with a filter through the relationship and with
//!   two fact-column filters, and a `SUMX` iterator.
//! - P12: completion on a synthetic model of 20 tables of 20 columns and 2 000 measures. One
//!   iteration is one editor request: `Schema::from_model` plus `complete` at the end of one of
//!   the [`COMPLETIONS`] texts, at most 100 items.
//!
//! ```text
//! cargo run --release -p pbi-dax --example perf_dax -- --scenario P04 --iterations 10
//! cargo run --release -p pbi-dax --example perf_dax -- --scenario P12 --iterations 60
//! ```

#[path = "../../../perf/harness.rs"]
mod harness;

use std::hint::black_box;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use arrow::array::{ArrayRef, Float64Array, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use harness::{Args, Outcome, ms_since, sample};
use pbi_dax::service::{Schema, complete};
use pbi_engine::{Engine, FilterContext, Value};
use pbi_model::{Column, ColumnRef, DataType, Measure, Model, Relationship, Table};

fn main() -> ExitCode {
    harness::main(
        env!("CARGO_MANIFEST_DIR"),
        &[
            ("P04", measures_over_1m_rows),
            ("P12", completion_on_2000_measures),
        ],
    )
}

const FACT_ROWS: usize = 1_000_000;
const PRODUCTS: i64 = 1_000;
const REGIONS: [&str; 6] = ["North", "South", "East", "West", "Central", "Online"];

/// `(name, expression, info key)` of the measures evaluated per iteration.
const MEASURES: [(&str, &str, &str); 5] = [
    ("Rows", "COUNTROWS(Sales)", "countrows_ms"),
    ("Amount", "SUM(Sales[Amount])", "sum_ms"),
    (
        "Category Amount",
        "CALCULATE([Amount], 'Product'[Category] = \"Category 03\")",
        "calculate_dimension_filter_ms",
    ),
    (
        "West Big Orders",
        "CALCULATE(COUNTROWS(Sales), Sales[Region] = \"West\", Sales[Quantity] > 50)",
        "calculate_fact_filters_ms",
    ),
    (
        "Revenue",
        "SUMX(Sales, Sales[Quantity] * Sales[Amount])",
        "sumx_ms",
    ),
];

fn measures_over_1m_rows(args: &Args) -> Result<Outcome, String> {
    let t = Instant::now();
    let model = model();
    let mut engine = Engine::new(&model);
    engine
        .load_tables([
            ("Sales".to_owned(), sales()?),
            ("Product".to_owned(), products()?),
        ])
        .map_err(|e| e.to_string())?;
    let snapshot = engine.snapshot();
    let setup_ms = ms_since(t);
    let filter = FilterContext::new();
    let mut per_measure = vec![Vec::new(); MEASURES.len()];
    let samples = sample(args.warmup, args.iterations, |_| {
        for ((name, _, _), times) in MEASURES.iter().zip(&mut per_measure) {
            let t = Instant::now();
            let v = pbi_dax::evaluate_measure(&snapshot, &model, name, &filter)
                .map_err(|e| format!("{name}: {e}"))?;
            times.push(ms_since(t));
            if matches!(v, Value::Blank) {
                return Err(format!("{name} returned BLANK"));
            }
            black_box(v);
        }
        Ok(())
    })?;
    let mut out = Outcome::ok(samples)
        .with("fact_rows", FACT_ROWS)
        .with("measures", MEASURES.len())
        .with("setup_ms", setup_ms);
    for ((_, _, key), times) in MEASURES.iter().zip(&mut per_measure) {
        // Per-measure medians include the warmup calls; they only show where time goes.
        times.sort_by(f64::total_cmp);
        out = out.with(key, times.get(times.len() / 2).copied().unwrap_or(f64::NAN));
    }
    Ok(out)
}

const COMPLETION_TABLES: usize = 20;
const COMPLETION_COLUMNS: usize = 20;
const COMPLETION_MEASURES: usize = 2_000;

/// Texts completed at their end, one per iteration in turn: a measure prefix, an empty table
/// argument (everything matches; tables rank first), a function prefix after variables, a
/// qualified column, a column argument, and a query whose `DEFINE` adds a measure.
const COMPLETIONS: [&str; 6] = [
    "[Measure 01",
    "SUMX(",
    "VAR total = [Measure 0001] RETURN CALCULATE(to",
    "'Table 05'[Col",
    "SUM(Col",
    "DEFINE MEASURE 'Table 01'[New] = 1 EVALUATE ROW(\"x\", [Ne",
];

fn completion_on_2000_measures(args: &Args) -> Result<Outcome, String> {
    let model = completion_model();
    let mut items = 0usize;
    let samples = sample(args.warmup, args.iterations, |i| {
        let text = COMPLETIONS
            .get(i % COMPLETIONS.len())
            .copied()
            .unwrap_or_default();
        let schema = Schema::from_model(&model);
        let c = complete(text, text.len(), &schema, 100);
        if c.items.is_empty() {
            return Err(format!("no completions for {text:?}"));
        }
        items += c.items.len();
        black_box(c);
        Ok(())
    })?;
    Ok(Outcome::ok(samples)
        .with("tables", COMPLETION_TABLES)
        .with("columns", COMPLETION_TABLES * COMPLETION_COLUMNS)
        .with("measures", COMPLETION_MEASURES)
        .with("items", items))
}

fn completion_model() -> Model {
    let per_table = COMPLETION_MEASURES / COMPLETION_TABLES;
    let tables = (1..=COMPLETION_TABLES)
        .map(|t| {
            let name = format!("Table {t:02}");
            let mut table = Table::new(&name);
            for c in 1..=COMPLETION_COLUMNS {
                table
                    .columns
                    .push(Column::data(format!("Column {c:02}"), DataType::Int64));
            }
            for m in 1..=per_table {
                let n = (t - 1) * per_table + m;
                table.measures.push(Measure::new(
                    format!("Measure {n:04}"),
                    format!("SUM('{name}'[Column {:02}])", (m % COMPLETION_COLUMNS) + 1),
                ));
            }
            table
        })
        .collect();
    Model {
        tables,
        ..Model::default()
    }
}

fn model() -> Model {
    let table = |name: &str, columns: &[(&str, DataType)]| {
        let mut t = Table::new(name);
        for (c, dt) in columns {
            t.columns.push(Column::data(*c, dt.clone()));
        }
        t
    };
    let mut sales = table(
        "Sales",
        &[
            ("ProductKey", DataType::Int64),
            ("Region", DataType::String),
            ("Quantity", DataType::Int64),
            ("Amount", DataType::Double),
        ],
    );
    for (name, expr, _) in MEASURES {
        sales.measures.push(Measure::new(name, expr));
    }
    let product = table(
        "Product",
        &[
            ("ProductKey", DataType::Int64),
            ("Category", DataType::String),
            ("Product", DataType::String),
        ],
    );
    Model {
        tables: vec![sales, product],
        relationships: vec![Relationship::new(
            "sales_product",
            ColumnRef::new("Sales", "ProductKey"),
            ColumnRef::new("Product", "ProductKey"),
        )],
        ..Model::default()
    }
}

/// Deterministic xorshift64 generator.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: i64) -> i64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        i64::try_from(self.0 % n.unsigned_abs().max(1)).unwrap_or(0)
    }
}

fn sales() -> Result<RecordBatch, String> {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut keys = Vec::with_capacity(FACT_ROWS);
    let mut regions = Vec::with_capacity(FACT_ROWS);
    let mut quantities = Vec::with_capacity(FACT_ROWS);
    let mut amounts = Vec::with_capacity(FACT_ROWS);
    for _ in 0..FACT_ROWS {
        keys.push(rng.below(PRODUCTS) + 1);
        regions.push(REGIONS[usize::try_from(rng.below(6)).unwrap_or(0) % REGIONS.len()]);
        quantities.push(rng.below(100) + 1);
        amounts.push(rng.below(100_000) as f64 / 100.0);
    }
    let columns: [(&str, ArrayRef); 4] = [
        ("ProductKey", Arc::new(Int64Array::from(keys))),
        ("Region", Arc::new(StringArray::from(regions))),
        ("Quantity", Arc::new(Int64Array::from(quantities))),
        ("Amount", Arc::new(Float64Array::from(amounts))),
    ];
    RecordBatch::try_from_iter(columns).map_err(|e| e.to_string())
}

fn products() -> Result<RecordBatch, String> {
    let keys: Vec<i64> = (1..=PRODUCTS).collect();
    let categories: Vec<String> = keys
        .iter()
        .map(|k| format!("Category {:02}", k % 12))
        .collect();
    let names: Vec<String> = keys.iter().map(|k| format!("Product {k:04}")).collect();
    let columns: [(&str, ArrayRef); 3] = [
        ("ProductKey", Arc::new(Int64Array::from(keys))),
        ("Category", Arc::new(StringArray::from(categories))),
        ("Product", Arc::new(StringArray::from(names))),
    ];
    RecordBatch::try_from_iter(columns).map_err(|e| e.to_string())
}
