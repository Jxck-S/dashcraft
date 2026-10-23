//! ABF backup container + `metadata.sqlitedb` (`pbi_vertipaq::fuzz_abf`).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    pbi_vertipaq::fuzz_abf(data);
});
