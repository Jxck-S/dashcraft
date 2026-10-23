//! VertiPaq `.idfmeta` / `.idf` / dictionary / Huffman decoders and the whole
//! PBIX data pipeline (`pbi_vertipaq::fuzz_vertipaq`).
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    pbi_vertipaq::fuzz_vertipaq(data);
});
