//! XPress9 / `DataModel` container decoding (`pbi_vertipaq::xpress9`).
//!
//! Invariants: never panics; outputs respect the limits; the one-shot and the
//! streaming (`Read`) decoders agree; uncompressed ABF input passes through
//! unchanged.
#![no_main]

use std::io::Read;

use libfuzzer_sys::fuzz_target;
use pbi_vertipaq::xpress9::{
    DataModelDecoder, DataModelFormat, Limits, decompress_datamodel, detect_format, fuzz_decompress,
};

fuzz_target!(|data: &[u8]| {
    fuzz_decompress(data);

    let limits = Limits::SMALL;
    let whole = decompress_datamodel(data, &limits);
    if let Ok(out) = &whole {
        assert!(out.len() as u64 <= limits.max_output_bytes);
        if detect_format(data) == Some(DataModelFormat::Uncompressed) {
            assert_eq!(out.as_slice(), data, "uncompressed ABF must pass through");
        }
    }

    let streamed = DataModelDecoder::new(data, limits).and_then(|mut decoder| {
        let mut out = Vec::new();
        decoder
            .read_to_end(&mut out)
            .map(|_| out)
            .map_err(pbi_vertipaq::Xpress9Error::from)
    });
    match (&whole, &streamed) {
        (Ok(a), Ok(b)) => assert!(a == b, "one-shot and streaming output differ"),
        (Err(_), Err(_)) => {}
        (a, b) => panic!(
            "one-shot and streaming decoders disagree: {:?} vs {:?}",
            a.as_ref().map(Vec::len),
            b.as_ref().map(Vec::len)
        ),
    }
});
