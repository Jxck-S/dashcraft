// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    match dashcraft_app_lib::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("dashcraft-app: failed to start the application: {err}");
            ExitCode::FAILURE
        }
    }
}
