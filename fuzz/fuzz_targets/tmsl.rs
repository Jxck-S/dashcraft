//! TMSL JSON (`model.bim`, PBIT `DataModelSchema`) reader and writer (`pbi_tmsl`).
//!
//! Invariants:
//! - never panics;
//! - an accepted document writes back as an equal JSON value (object key
//!   order and number spelling aside — the documented round-trip guarantee);
//! - the source-layout writer's bytes read back to the same model JSON and
//!   re-write byte for byte;
//! - the canonical `write_bim` output is a fixed point.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_tmsl::{
    ReadOptions, decode_text, read_bim, read_bytes, read_data_model_schema, to_json, write_bim,
    write_bytes,
};
use serde_json::Value;

/// JSON equality ignoring object key order and number spelling (`1E5` vs
/// `1e5`; numbers keep their source text under `arbitrary_precision`).
fn equivalent(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, v)| b.get(k).is_some_and(|w| equivalent(v, w)))
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(v, w)| equivalent(v, w))
        }
        (Value::Number(a), Value::Number(b)) => {
            a.to_string().eq_ignore_ascii_case(&b.to_string())
                || matches!((a.as_f64(), b.as_f64()), (Some(x), Some(y)) if x == y)
        }
        _ => a == b,
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(loaded) = read_bytes(data, &ReadOptions::default()) else {
        return;
    };
    let options = loaded.write_options();
    let json = to_json(&loaded.model, &options);

    if !loaded.diagnostics.iter().any(|d| d.code == "TMSL001")
        && let Ok(decoded) = decode_text(data)
        && let Ok(original) = serde_json::from_str::<Value>(&decoded.text)
    {
        assert!(
            equivalent(&json, &original),
            "to_json(read(x)) != x\noriginal: {original}\nwritten:  {json}"
        );
    }

    let written = write_bytes(&loaded.model, &options);
    let again = match read_data_model_schema(&written) {
        Ok(again) => again,
        Err(e) => panic!(
            "writer output does not read back: {e}\n{}",
            String::from_utf8_lossy(&written)
        ),
    };
    let again_options = again.write_options();
    assert!(
        to_json(&again.model, &again_options) == json,
        "model JSON changed through the writer"
    );
    let rewritten = write_bytes(&again.model, &again_options);
    if rewritten != written {
        let at = rewritten
            .iter()
            .zip(&written)
            .position(|(a, b)| a != b)
            .unwrap_or(rewritten.len().min(written.len()));
        let context = |b: &[u8]| {
            String::from_utf8_lossy(&b[at.saturating_sub(80)..(at + 80).min(b.len())]).into_owned()
        };
        panic!(
            "source-layout writer is not a fixed point (byte {at}; {:?} vs {:?})\nfirst:  {:?}\nsecond: {:?}",
            options.format,
            again_options.format,
            context(&written),
            context(&rewritten)
        );
    }

    let bim = write_bim(
        &loaded.model,
        loaded.database_name.as_deref(),
        loaded.compatibility_level,
    );
    let reread = match read_bim(&bim) {
        Ok(r) => r,
        Err(e) => panic!("write_bim output does not read back: {e}\n{bim}"),
    };
    assert_eq!(
        write_bim(
            &reread.model,
            reread.database_name.as_deref(),
            reread.compatibility_level
        ),
        bim,
        "write_bim is not a fixed point"
    );
});
