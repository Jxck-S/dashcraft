//! End-to-end tests of `dashcraft lint` over the synthetic Acme Outfitters
//! example, a copy with one stale `queryRef` and a deliberately broken copy.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use common::{ACME, KPI_CARD, acme_with_stale_query_ref, copy_dir, fixture_root};
use serde_json::Value;

fn pbiview_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_dashcraft"))
        .args(args)
        .current_dir(dir)
        .env_remove("RUST_LOG")
        .output()
        .expect("run pbiview")
}

fn pbiview(args: &[&str]) -> Output {
    pbiview_in(&fixture_root(), args)
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).expect("UTF-8 stdout")
}

fn assert_exit(o: &Output, code: i32) {
    assert_eq!(
        o.status.code(),
        Some(code),
        "stdout:\n{}\nstderr:\n{}",
        stdout(o),
        String::from_utf8_lossy(&o.stderr)
    );
}

fn json(o: &Output) -> Value {
    serde_json::from_str(&stdout(o)).expect("valid JSON")
}

fn have_example() -> bool {
    let ok = fixture_root().join(ACME).exists();
    if !ok {
        eprintln!("SKIP: {ACME} missing");
    }
    ok
}

fn tmp_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("pbiview-cli-lint-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap()
}

/// `<tmp>/acme`: the example with one stale `queryRef` (relative paths keep the
/// output machine-independent).
fn stale_copy() -> (tempfile::TempDir, PathBuf) {
    let dir = tmp_dir();
    acme_with_stale_query_ref(&dir.path().join("acme"));
    let root = dir.path().to_owned();
    (dir, root)
}

#[test]
fn lint_text_groups_findings_by_file() {
    if !have_example() {
        return;
    }
    let o = pbiview(&["lint", ACME]);
    assert_exit(&o, 0);
    assert_eq!(stdout(&o), format!("{ACME}: no problems found\n"));

    let (_dir, root) = stale_copy();
    let o = pbiview_in(&root, &["lint", "acme"]);
    assert_exit(&o, 0);
    let out = stdout(&o);
    assert!(out.contains(&format!("{KPI_CARD}\n")), "{out}");
    assert!(
        out.contains("  warning LINT003 #/visual/query/queryState/Data/projections/0/queryRef "),
        "{out}"
    );
    assert!(
        out.contains("      hint: set queryRef to `Orders.Total Sales`"),
        "{out}"
    );
    assert!(
        out.ends_with("acme: 0 errors, 1 warning, 0 infos\n"),
        "{out}"
    );

    // Warnings fail the run only when denied.
    assert_exit(
        &pbiview_in(&root, &["lint", "acme", "--deny", "warnings"]),
        2,
    );
    let quiet = pbiview_in(
        &root,
        &["lint", "acme", "--deny", "warnings", "--disable", "LINT003"],
    );
    assert_exit(&quiet, 0);
    assert_eq!(stdout(&quiet), "acme: no problems found\n");
}

#[test]
fn lint_json_is_stable() {
    if !have_example() {
        return;
    }
    let o = pbiview(&["lint", "--json", ACME]);
    assert_exit(&o, 0);
    let v = json(&o);
    assert_eq!(v["schemaVersion"], 1);
    assert_eq!(v["path"], ACME);
    assert_eq!(
        (&v["summary"]["errors"], &v["summary"]["warnings"]),
        (&Value::from(0), &Value::from(0))
    );
    assert_eq!(v["findings"], Value::Array(Vec::new()));

    let (_dir, root) = stale_copy();
    let o = pbiview_in(&root, &["lint", "--json", "acme"]);
    assert_exit(&o, 0);
    let v = json(&o);
    assert_eq!(v["path"], "acme");
    assert_eq!(v["summary"]["errors"], 0);
    assert_eq!(v["summary"]["warnings"], 1);
    assert_eq!(v["summary"]["byRule"]["LINT003"], 1);
    assert_eq!(v["readerErrors"], Value::Array(Vec::new()));
    let f = &v["findings"][0];
    assert_eq!(f["rule"], "LINT003");
    assert_eq!(f["severity"], "warning");
    assert_eq!(
        f["origin"]["pointer"],
        "/visual/query/queryState/Data/projections/0/queryRef"
    );
    assert_eq!(f["suggestion"], "set queryRef to `Orders.Total Sales`");
}

#[test]
fn lint_broken_project_exits_with_2() {
    if !have_example() {
        return;
    }
    let dir = tmp_dir();
    let root = dir.path().join("acme");
    copy_dir(&fixture_root().join(ACME), &root);
    let tmdl = root.join("AcmeOutfitters.SemanticModel/definition/tables/Stores.tmdl");
    let text = fs::read_to_string(&tmdl).unwrap();
    let renamed = text.replace("\tcolumn Channel\n", "\tcolumn 'Sales Channel'\n");
    assert_ne!(renamed, text);
    fs::write(&tmdl, renamed).unwrap();

    let o = pbiview(&["lint", "--json", root.to_str().unwrap()]);
    assert_exit(&o, 2);
    let v = json(&o);
    // The donut chart's category (LINT002) and the report filters of the
    // Stores and Online reports (LINT004).
    assert_eq!(v["summary"]["errors"], 3);
    assert_eq!(v["summary"]["byRule"]["LINT002"], 1);
    assert_eq!(v["summary"]["byRule"]["LINT004"], 2);
    let missing: Vec<&Value> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["rule"] == "LINT002")
        .collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(
        missing[0]["suggestion"],
        "did you mean `Stores[Sales Channel]`?"
    );

    let text = stdout(&pbiview(&["lint", root.to_str().unwrap()]));
    assert!(text.contains("  error LINT002 "), "{text}");
    assert!(text.contains("  error LINT004 "), "{text}");
}

#[test]
fn lint_missing_path_is_fatal() {
    let o = pbiview(&["lint", "examples/does-not-exist"]);
    assert_exit(&o, 1);
}
