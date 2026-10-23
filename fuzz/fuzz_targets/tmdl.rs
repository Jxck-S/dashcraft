//! TMDL: lossless CST, model mapping and the writers (`pbi_tmdl`).
//!
//! Input: one or more `.tmdl` files separated by NUL bytes.
//!
//! Invariants (as in `crates/pbi-tmdl/tests/robustness.rs`):
//! - a file parses or fails with a syntax error, and the CST re-emits the
//!   input byte for byte;
//! - the canonical writer's output parses and maps to the same model
//!   (`load(write(m)) == m`, origins aside);
//! - so does the preserving writer's output.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_model::Model;
use pbi_tmdl::{
    TmdlError, clear_origins, load_model, parse_file, write_model, write_model_preserving,
};

const MAX_FILES: usize = 8;

fn normalized(mut model: Model) -> Model {
    clear_origins(&mut model);
    if let Some(db) = &mut model.database {
        db.origin = None;
    }
    model
}

fuzz_target!(|data: &[u8]| {
    let Ok(text) = std::str::from_utf8(data) else {
        return;
    };
    let files: Vec<(String, String)> = text
        .split('\0')
        .take(MAX_FILES)
        .enumerate()
        .map(|(i, t)| (format!("f{i}.tmdl"), t.to_owned()))
        .collect();
    for (path, text) in &files {
        match parse_file(path, text) {
            Ok(file) => assert_eq!(&file.to_string(), text, "CST is not lossless"),
            Err(TmdlError::Syntax { .. }) => return,
            Err(e) => panic!("unexpected error kind: {e}"),
        }
    }
    let loaded = match load_model(&files) {
        Ok(loaded) => loaded,
        Err(e) => panic!("every file parsed, but load_model failed: {e}"),
    };

    let written = write_model(&loaded.model, loaded.database.as_ref());
    let reloaded = match load_model(&written) {
        Ok(r) => r,
        Err(e) => panic!("writer output does not parse: {e}\nwritten: {written:#?}"),
    };
    let preserved = write_model_preserving(&loaded.model, loaded.database.as_ref(), &loaded.files);
    let preserved = match load_model(&preserved) {
        Ok(r) => r,
        Err(e) => panic!("preserving writer output does not parse: {e}\nwritten: {preserved:#?}"),
    };

    let model = normalized(loaded.model);
    assert!(
        model == normalized(preserved.model),
        "preserving writer changed the model"
    );
    let reloaded = normalized(reloaded.model);
    assert!(
        model == reloaded,
        "model changed through the canonical writer\nwritten: {written:#?}\nbefore: {model:#?}\nafter: {reloaded:#?}"
    );
});
