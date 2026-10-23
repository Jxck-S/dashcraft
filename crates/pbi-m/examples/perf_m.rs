//! Performance scenario P05 (`cargo xtask perf`; budgets in `perf/budgets.toml`, docs in
//! `docs/perf.md`): an M partition query over a synthetic 100 000-row CSV, served from memory
//! through a [`BlobResolver`] like `AzureStorage.Blobs` in the Acme Outfitters example. One
//! iteration evaluates `Csv.Document` → `Table.PromoteHeaders` → `Table.TransformColumnTypes` →
//! `Table.SelectRows` → `Table.AddColumn` and converts the result to Arrow.
//!
//! ```text
//! cargo run --release -p pbi-m --example perf_m -- --scenario P05 --iterations 10
//! ```

#[path = "../../../perf/harness.rs"]
mod harness;

use std::fmt::Write as _;
use std::hint::black_box;
use std::process::ExitCode;
use std::sync::Arc;

use harness::{Args, Outcome, sample};
use indexmap::IndexMap;
use pbi_m::{BlobEntry, BlobResolver, Engine, EvalOptions, ResolverError, ResolverErrorKind};

fn main() -> ExitCode {
    harness::main(
        env!("CARGO_MANIFEST_DIR"),
        &[("P05", csv_select_add_column)],
    )
}

const ROWS: usize = 100_000;
const CONTAINER: &str = "https://perf.blob.core.windows.net/data";
const BLOB: &str = "sales/sales.csv";
const REGIONS: [&str; 6] = ["North", "South", "East", "West", "Central", "Online"];

const QUERY: &str = r#"
let
    Source = AzureStorage.Blobs("https://perf.blob.core.windows.net/data"),
    Csv = Csv.Document(Source{[Name = "sales/sales.csv"]}[Content], [Delimiter = ",", Encoding = 65001, QuoteStyle = QuoteStyle.Csv]),
    Promoted = Table.PromoteHeaders(Csv, [PromoteAllScalars = true]),
    Typed = Table.TransformColumnTypes(Promoted, {{"Order ID", Int64.Type}, {"Order Date", type date}, {"Quantity", Int64.Type}, {"Amount", type number}}),
    Filtered = Table.SelectRows(Typed, each [Quantity] > 10 and [Region] <> "North"),
    Added = Table.AddColumn(Filtered, "Line Total", each [Quantity] * [Amount], type number)
in
    Added
"#;

/// Serves one in-memory CSV blob.
#[derive(Debug)]
struct MemoryBlobs {
    csv: Arc<[u8]>,
}

impl BlobResolver for MemoryBlobs {
    fn list(&self, container_url: &str) -> Result<Vec<BlobEntry>, ResolverError> {
        if container_url != CONTAINER {
            return Err(ResolverError::new(
                ResolverErrorKind::NotFound,
                format!("unknown container {container_url}"),
            ));
        }
        Ok(vec![BlobEntry {
            name: BLOB.to_owned(),
            size: u64::try_from(self.csv.len()).ok(),
            modified: None,
        }])
    }

    fn open(&self, _container_url: &str, name: &str) -> Result<Vec<u8>, ResolverError> {
        if name != BLOB {
            return Err(ResolverError::new(
                ResolverErrorKind::NotFound,
                format!("unknown blob {name}"),
            ));
        }
        Ok(self.csv.to_vec())
    }
}

fn csv_select_add_column(args: &Args) -> Result<Outcome, String> {
    let csv = sales_csv();
    let csv_kb = csv.len() / 1024;
    let engine = Engine::new(EvalOptions {
        resolver: Arc::new(MemoryBlobs { csv: csv.into() }),
        ..EvalOptions::default()
    });
    let shared = IndexMap::new();
    let mut rows = 0;
    let samples = sample(args.warmup, args.iterations, |_| {
        let batch = engine
            .evaluate_partition_to_arrow(QUERY, &shared, &[])
            .map_err(|e| e.to_string())?;
        rows = black_box(batch).num_rows();
        Ok(())
    })?;
    if rows == 0 {
        return Err("the query returned no rows".to_owned());
    }
    Ok(Outcome::ok(samples)
        .with("csv_rows", ROWS)
        .with("csv_kb", csv_kb)
        .with("result_rows", rows))
}

/// Deterministic xorshift64 generator.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

fn sales_csv() -> Vec<u8> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut out = String::with_capacity(ROWS * 80);
    out.push_str("Order ID,Customer,Product,Category,Region,Order Date,Quantity,Amount\n");
    for i in 1..=ROWS {
        let product = rng.below(200);
        let region = REGIONS[usize::try_from(rng.below(6)).unwrap_or(0) % REGIONS.len()];
        let _ = writeln!(
            out,
            "{i},\"Customer {:05}\",Product {product:03},Category {:02},{region},2025-{:02}-{:02},{},{}.{:02}",
            rng.below(5_000),
            product % 12,
            rng.below(12) + 1,
            rng.below(28) + 1,
            rng.below(100) + 1,
            rng.below(10_000),
            rng.below(100),
        );
    }
    out.into_bytes()
}
