//! Performance scenario P06 (`cargo xtask perf`; budgets in `perf/budgets.toml`, docs in
//! `docs/perf.md`): opening the data of a PBIX, i.e. XPress9 decompression, the ABF backup,
//! `metadata.sqlitedb` and VertiPaq column decoding to Arrow ([`pbi_vertipaq::open_pbix_data`]).
//! It reads the `DataModel` stream of the committed Microsoft sample
//! `reference/samples/microsoft/Revenue Opportunities.pbix` (MIT) and is skipped without it.
//! Only sizes are reported.
//!
//! ```text
//! cargo run --release -p pbi-vertipaq --example perf_vertipaq -- --scenario P06 --iterations 10
//! ```

#[path = "../../../perf/harness.rs"]
mod harness;

use std::hint::black_box;
use std::process::ExitCode;

use harness::{Args, Outcome, sample};
use pbi_vertipaq::{DecodeLimits, open_pbix_data};

fn main() -> ExitCode {
    harness::main(env!("CARGO_MANIFEST_DIR"), &[("P06", pbix_data_open)])
}

fn pbix_data_open(args: &Args) -> Result<Outcome, String> {
    let path = args
        .reference
        .join("samples/microsoft/Revenue Opportunities.pbix");
    let Ok(zip) = std::fs::read(&path) else {
        return Ok(Outcome::missing(&path));
    };
    let bytes = stored_zip_entry(&zip, "DataModel")
        .ok_or_else(|| format!("{}: no stored DataModel entry", path.display()))?;
    let limits = DecodeLimits::default();
    let (mut tables, mut columns) = (0, 0);
    let samples = sample(args.warmup, args.iterations, |_| {
        let (_model, decoded, _diagnostics) =
            open_pbix_data(bytes, &limits).map_err(|e| e.to_string())?;
        tables = decoded.len();
        columns = decoded.iter().map(|(_, b)| b.num_columns()).sum();
        black_box(decoded);
        Ok(())
    })?;
    Ok(Outcome::ok(samples)
        .with("datamodel_kb", bytes.len() / 1024)
        .with("tables", tables)
        .with("columns", columns))
}

/// A stored (uncompressed) ZIP entry, found through the central directory
/// (Power BI stores `DataModel`: it is already XPress9-compressed).
fn stored_zip_entry<'a>(zip: &'a [u8], name: &str) -> Option<&'a [u8]> {
    let u16_at = |at: usize| -> Option<usize> {
        let b = zip.get(at..at.checked_add(2)?)?;
        Some(usize::from(u16::from_le_bytes([b[0], b[1]])))
    };
    let u32_at = |at: usize| -> Option<usize> {
        let b = zip.get(at..at.checked_add(4)?)?;
        usize::try_from(u32::from_le_bytes([b[0], b[1], b[2], b[3]])).ok()
    };
    let eocd = (0..=zip.len().checked_sub(22)?)
        .rev()
        .find(|&i| zip.get(i..i + 4) == Some(&[0x50, 0x4b, 0x05, 0x06]))?;
    let mut at = u32_at(eocd + 16)?;
    for _ in 0..u16_at(eocd + 10)? {
        if zip.get(at..at.checked_add(4)?)? != [0x50, 0x4b, 0x01, 0x02] {
            return None;
        }
        let (method, size, name_len) = (u16_at(at + 10)?, u32_at(at + 20)?, u16_at(at + 28)?);
        let skip = name_len + u16_at(at + 30)? + u16_at(at + 32)?;
        if zip.get(at + 46..at + 46 + name_len)? == name.as_bytes() {
            if method != 0 {
                return None;
            }
            let local = u32_at(at + 42)?;
            let start = local + 30 + u16_at(local + 26)? + u16_at(local + 28)?;
            return zip.get(start..start.checked_add(size)?);
        }
        at = at.checked_add(46 + skip)?;
    }
    None
}
