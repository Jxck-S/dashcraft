#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Untrusted input: malformed, wrong-typed and deeply nested JSON must
//! produce errors or diagnostics, never panics.

use pbi_pbir::{
    MemoryFileSource, PbirError, QueryExpr, ThemeJson, VisualContainerJson, codes,
    legacy_to_report_ast, read_legacy_layout, read_pbir, to_report_ast, write_pbir,
};

const PAGE: &str =
    r#"{"name":"p1","displayName":"P","displayOption":"FitToPage","height":720,"width":1280}"#;

#[test]
fn not_a_report_is_an_error() {
    let empty = MemoryFileSource::new();
    assert!(matches!(read_pbir(&empty), Err(PbirError::NotPbir(_))));
    let other: MemoryFileSource = [("model.bim", "{}")].into_iter().collect();
    assert!(matches!(read_pbir(&other), Err(PbirError::NotPbir(_))));
}

#[test]
fn invalid_files_become_diagnostics_and_are_kept_verbatim() {
    let src: MemoryFileSource = [
        ("definition/report.json", r#"{"themeCollection": 5}"#),
        ("definition/pages/pages.json", r#"{"pageOrder":["p1","gone"]}"#),
        ("definition/pages/p1/page.json", PAGE),
        ("definition/pages/p1/visuals/v1/visual.json", "{not json"),
        (
            "definition/pages/p1/visuals/v2/visual.json",
            r#"{"name":"v2","position":{"x":"left"}}"#,
        ),
        (
            "definition/pages/p1/visuals/v3/visual.json",
            r#"{"name":"v3","position":{"x":1,"y":2,"z":0,"width":3,"height":4},"visual":{"visualType":"shape"}}"#,
        ),
        ("definition/pages/p2/page.json", r#"{"name":"p2","width":[]}"#),
        ("definition/pages/p3/page.json", r#"{"name":"p3"}"#),
        ("definition/pages/orphan/visuals/v/visual.json", "{}"),
    ]
    .into_iter()
    .collect();
    let report = read_pbir(&src).unwrap();
    let codes_seen: Vec<_> = report.diagnostics.iter().map(|d| d.code.as_str()).collect();
    assert_eq!(
        codes_seen,
        [
            codes::INVALID_FILE,  // report.json
            codes::INVALID_FILE,  // v1
            codes::INVALID_FILE,  // v2
            codes::INVALID_FILE,  // p2
            codes::MISSING_FILE,  // orphan visuals
            codes::MISSING_FILE,  // "gone" in pageOrder
            codes::UNLISTED_PAGE, // p3
        ],
        "{:#?}",
        report.diagnostics
    );
    assert!(
        report.diagnostics[2]
            .message
            .contains("position: x: invalid type")
    );
    assert_eq!(report.pages.len(), 2);
    assert_eq!(report.pages[0].visuals.len(), 1);
    let (ast, _) = to_report_ast(&report);
    assert_eq!(ast.pages[0].visuals[0].visual_type, "shape");

    // Invalid files are written back untouched; valid ones re-serialize
    // to the same (compact) bytes.
    let written = write_pbir(&report);
    assert_eq!(written.len(), src.files.len());
    for (path, bytes) in written {
        assert_eq!(bytes, src.files[&path], "{path}");
    }
}

#[test]
fn wrong_types_are_errors() {
    for json in [
        r#"{"visual":{"query":{"queryState":{"Values":{"projections":[{"field":{"Column":{"Property":7}}}]}}}}}"#,
        r#"{"filterConfig":{"filters":[{"isHiddenInViewMode":"yes"}]}}"#,
        r#"{"position":[]}"#,
        r#"[]"#,
        r#""visual""#,
    ] {
        assert!(
            serde_json::from_str::<VisualContainerJson>(json).is_err(),
            "{json}"
        );
    }
    assert!(serde_json::from_str::<ThemeJson>(r##"{"dataColors":["#fff",1]}"##).is_err());
}

fn nested_not(depth: usize) -> String {
    let leaf = r#"{"Literal":{"Value":"true"}}"#;
    let mut s = String::new();
    for _ in 0..depth {
        s.push_str(r#"{"Not":{"Expression":"#);
    }
    s.push_str(leaf);
    for _ in 0..depth {
        s.push_str("}}");
    }
    s
}

#[test]
fn deep_nesting_hits_the_parser_limit_instead_of_overflowing() {
    // serde_json limits nesting to 128 levels; each `Not` adds two.
    let ok = nested_not(60);
    let expr: QueryExpr = serde_json::from_str(&ok).unwrap();
    assert!(matches!(expr.kind(), pbi_pbir::ExprKind::Not(_)));
    assert!(serde_json::from_str::<QueryExpr>(&nested_not(10_000)).is_err());

    // A deep but valid condition converts without blowing the stack.
    let filter = format!(
        r#"{{"filterConfig":{{"filters":[{{"name":"f","filter":{{"From":[],"Where":[{{"Condition":{}}}]}}}}]}}}}"#,
        nested_not(58)
    );
    let src: MemoryFileSource = [
        ("definition/report.json", filter.as_str()),
        ("definition/pages/p1/page.json", PAGE),
    ]
    .into_iter()
    .collect();
    let report = read_pbir(&src).unwrap();
    assert!(report.diagnostics.is_empty(), "{:?}", report.diagnostics);
    let (ast, _) = to_report_ast(&report);
    assert!(ast.filters[0].condition.is_some());

    let too_deep: MemoryFileSource = [
        ("definition/report.json", nested_not(5_000)),
        ("definition/pages/p1/page.json", PAGE.to_owned()),
    ]
    .into_iter()
    .collect();
    let report = read_pbir(&too_deep).unwrap();
    assert_eq!(report.diagnostics[0].code, codes::INVALID_FILE);
}

#[test]
fn bad_legacy_layouts_are_errors() {
    let cases: [&[u8]; 7] = [
        b"",
        b"not json",
        b"{\"sections\": 5}",
        b"{\"config\": \"{}\"}",
        b"\xFF\xFE\x7B",
        b"\xFF\xFE\x00\xD8\x7B\x00",
        b"[1,2,3]",
    ];
    for bytes in cases {
        assert!(read_legacy_layout(bytes).is_err(), "{bytes:?}");
    }
    let deep = format!(r#"{{"sections":[],"config":{:?}}}"#, "[".repeat(10_000));
    let legacy = read_legacy_layout(deep.as_bytes()).unwrap();
    assert_eq!(legacy.diagnostics[0].code, codes::INVALID_EMBEDDED_JSON);

    // Wrong-typed embedded parts degrade to diagnostics.
    let odd = r#"{"sections":[{"name":"s","visualContainers":[{"config":"{\"name\":5}","filters":"{}"},{"x":"wide"}]}]}"#;
    assert!(read_legacy_layout(odd.as_bytes()).is_err());
    let odd = r#"{"sections":[{"name":"s","displayOption":"weird","visualContainers":[{"config":"{\"name\":5}","filters":"{}","query":"nope"}]}]}"#;
    let legacy = read_legacy_layout(odd.as_bytes()).unwrap();
    assert_eq!(legacy.diagnostics.len(), 3);
    let (report, _) = legacy_to_report_ast(&legacy);
    assert_eq!(report.pages[0].visuals.len(), 1);
    assert!(report.pages[0].display_option.is_unknown());
}
