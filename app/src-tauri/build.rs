// On Windows (MSVC) the app manifest (Common Controls v6) is embedded through linker arguments
// rather than by tauri-build, so it reaches the test binaries too: without it they import
// `TaskDialogIndirect` from comctl32 v5 and die at load with STATUS_ENTRYPOINT_NOT_FOUND
// (tauri-apps/tauri#13419). The lib is an rlib, so only the binary and the tests link.
fn main() {
    let mut attributes = tauri_build::Attributes::new();
    let windows_msvc = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows_msvc {
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        let manifest =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
    }
    if let Err(error) = tauri_build::try_build(attributes) {
        eprintln!("tauri-build failed: {error:#}");
        std::process::exit(1);
    }
}
