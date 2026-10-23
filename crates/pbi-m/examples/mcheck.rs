//! Parses M files and checks that printing round-trips.
//!
//! `cargo run -p pbi-m --example mcheck -- file.pq [more files...]`

use std::process::ExitCode;

fn check(path: &str) -> Result<(), String> {
    let src = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut doc = pbi_m::parse_document(&src).map_err(|e| e.render(&src))?;
    let printed = pbi_m::print_document(&doc);
    let mut reparsed = pbi_m::parse_document(&printed)
        .map_err(|e| format!("printed output does not parse: {}", e.render(&printed)))?;
    doc.clear_spans();
    reparsed.clear_spans();
    if doc != reparsed {
        return Err("printed output parses to a different tree".to_owned());
    }
    Ok(())
}

fn main() -> ExitCode {
    let mut failed = false;
    for path in std::env::args().skip(1) {
        match check(&path) {
            Ok(()) => println!("ok    {path}"),
            Err(e) => {
                failed = true;
                println!("FAIL  {path}: {e}");
            }
        }
    }
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
