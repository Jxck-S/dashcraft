//! Power Query M evaluation under tight limits (`pbi_m::Engine`).
//!
//! Input: the query, optionally followed by NUL-separated shared queries
//! named `Q0`, `Q1`, ... (so the query can reference them).
//!
//! Invariants: never panics (on the evaluator thread either — the fuzzer's
//! panic hook aborts on any thread); never reports an internal error; no
//! file, network or blob access (`NoBlobResolver`).
#![no_main]

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use chrono::DateTime;
use indexmap::IndexMap;
use libfuzzer_sys::fuzz_target;
use pbi_m::{Engine, EvalOptions, FixedClock, Limits, MError, NoBlobResolver};

const MAX_SHARED: usize = 4;

fn engine() -> Engine {
    let now = DateTime::from_timestamp(1_792_540_800, 0).unwrap_or_default();
    Engine::new(EvalOptions {
        resolver: Arc::new(NoBlobResolver),
        clock: Arc::new(FixedClock::utc(now)),
        limits: Limits {
            max_rows: 256,
            max_steps: 20_000,
            max_depth: 64,
            max_binary_bytes: 64 * 1024,
        },
        cancel: Arc::new(AtomicBool::new(false)),
    })
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let mut parts = text.split('\0');
    let src = parts.next().unwrap_or_default();
    let shared: IndexMap<String, String> = parts
        .take(MAX_SHARED)
        .enumerate()
        .map(|(i, q)| (format!("Q{i}"), q.to_owned()))
        .collect();

    // Evaluates and resolves the whole value, then converts tables to Arrow.
    if let Err(MError::Internal(message)) = engine().evaluate_partition_to_arrow(src, &shared, &[])
    {
        panic!("internal error: {message}\nsource: {src:?}");
    }
});
