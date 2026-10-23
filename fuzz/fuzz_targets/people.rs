//! People-picker cells exported as Python-style lists (`pbi_session::transforms::people`).
//!
//! Invariants: never panics; unparsed text is the trimmed input; names are
//! non-empty and trimmed; the display helper agrees with `People::display`.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_session::transforms::people::{People, parse_people, parse_people_display};

fuzz_target!(|data: &[u8]| {
    let Ok(raw) = std::str::from_utf8(data) else {
        return;
    };
    let people = parse_people(raw);
    match &people {
        People::Empty => {}
        People::Unparsed(text) => assert_eq!(text, raw.trim()),
        People::Names(names) => {
            assert!(!names.is_empty());
            for name in names {
                assert!(!name.is_empty());
                assert_eq!(name, name.trim());
            }
        }
    }
    assert_eq!(parse_people_display(raw), people.display());
});
