#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Legacy layout (PBIX `Report/Layout`, PBIR-Legacy `report.json`) →
//! report AST, using a synthetic layout that mirrors part of the Acme
//! Outfitters Stores report (`tests/data/legacy_layout.json`).

mod common;

use pbi_model::{
    AggregationFunction, ComparisonKind, Condition, DisplayOption, FieldExpr, FilterKind,
    InteractionKind, Literal, Report, SortDirection,
};
use pbi_pbir::{
    LegacyReport, TextEncoding, codes, legacy_to_report_ast, read_legacy_layout,
    read_legacy_layout_at, read_pbir, to_report_ast,
};

const LAYOUT: &str = include_str!("data/legacy_layout.json");
const PERFORMANCE: &str = "ef3ce6b6fadf22960137";
const CARD: &str = "c81376feb163c3047446";
const SLICER: &str = "ba3b1b7c5a9cede8452c";
const TABLE: &str = "d4431298c2010cc0f494";
const TEXTBOX: &str = "1279aa9cd39ad01e1554";

fn utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

fn convert(legacy: &LegacyReport) -> Report {
    assert!(legacy.diagnostics.is_empty(), "{:?}", legacy.diagnostics);
    let (report, diags) = legacy_to_report_ast(legacy);
    assert!(diags.is_empty(), "{diags:?}");
    report
}

#[test]
fn utf8_and_utf16le_layouts_give_the_same_ast() {
    let utf8 = read_legacy_layout(LAYOUT.as_bytes()).unwrap();
    assert_eq!(utf8.encoding, TextEncoding::Utf8);
    let utf16 = read_legacy_layout(&utf16le(LAYOUT)).unwrap();
    assert_eq!(utf16.encoding, TextEncoding::Utf16Le);
    assert_eq!(convert(&utf8), convert(&utf16));
    let original: serde_json::Value = serde_json::from_str(LAYOUT).unwrap();
    let back = serde_json::to_value(&utf16.layout).unwrap();
    assert!(pbi_pbir::json_identical(&back, &original));
}

#[test]
fn legacy_layout_converts_like_pbir() {
    let report = convert(&read_legacy_layout_at(LAYOUT.as_bytes(), "Report/Layout").unwrap());

    let base = report.theme.base_theme.as_ref().unwrap();
    assert_eq!(base.name, "CY24SU10");
    assert_eq!(base.kind.as_deref(), Some("SharedResources"));
    assert_eq!(
        report.static_resource_path(base),
        Some("StaticResources/SharedResources/BaseThemes/CY24SU10.json".into())
    );
    let custom = report.theme.custom_theme.as_ref().unwrap();
    assert_eq!(custom.name, "AcmeOutfitters.json");
    assert_eq!(custom.kind.as_deref(), Some("RegisteredResources"));
    assert_eq!(
        report.static_resource_path(custom),
        Some("StaticResources/RegisteredResources/AcmeOutfitters.json".into())
    );
    assert_eq!(report.active_page.as_deref(), Some(PERFORMANCE));

    let names: Vec<_> = report
        .pages
        .iter()
        .map(|p| p.display_name.as_str())
        .collect();
    assert_eq!(names, ["Store Performance", "Store Detail"]);
    let main = &report.pages[0];
    let detail = &report.pages[1];
    assert_eq!((main.width, main.height), (1280.0, 720.0));
    assert_eq!((detail.width, detail.height), (1280.0, 900.0));
    assert_eq!(main.display_option, DisplayOption::FitToPage);
    assert_eq!(detail.display_option, DisplayOption::FitToWidth);
    assert!(detail.is_hidden && !main.is_hidden);
    assert_eq!(main.visuals.len(), 4);
    assert_eq!(detail.visuals.len(), 1);
    assert_eq!(
        main.interaction(SLICER, CARD),
        Some(&InteractionKind::DataFilter)
    );
    assert_eq!(
        main.interaction(SLICER, TEXTBOX),
        Some(&InteractionKind::NoFilter)
    );

    // Report filter.
    let f = &report.filters[0];
    assert_eq!(f.name, "b0d63946d6634cc987ce");
    assert_eq!(f.kind, FilterKind::Categorical);
    assert!(!f.is_hidden_in_view_mode && !f.is_locked_in_view_mode);
    assert_eq!(f.how_created.as_deref(), Some("User"));
    assert_eq!(f.field, Some(FieldExpr::column("Stores", "Channel")));
    assert_eq!(
        f.condition,
        Some(Condition::In {
            exprs: vec![FieldExpr::column("Stores", "Channel")],
            values: vec![vec![Literal::text("Retail")], vec![Literal::text("Outlet")]],
        })
    );
    let origin = f.origin.as_ref().unwrap();
    assert_eq!(origin.to_string(), "Report/Layout#/filters/0");

    // Page filter, hidden and locked.
    let pf = &detail.filters[0];
    assert!(pf.is_hidden_in_view_mode && pf.is_locked_in_view_mode);
    assert_eq!(
        pf.condition,
        Some(Condition::In {
            exprs: vec![FieldExpr::column("Stores", "Region")],
            values: vec![vec![Literal::text("East")]],
        })
    );

    // Card: measure resolved through prototypeQuery alias `o`.
    let card = main.visual(CARD).unwrap();
    assert_eq!(card.visual_type, "cardVisual");
    assert_eq!(card.position.x, 280.25);
    assert_eq!(card.position.width, 975.5);
    assert_eq!(card.position.tab_order, Some(6000));
    let p = &card.projections[0];
    assert_eq!(p.role, "Data");
    assert_eq!(p.field, FieldExpr::measure("Orders", "Total Sales"));
    assert_eq!(p.query_ref, "Orders.Total Sales");
    assert_eq!(p.native_query_ref.as_deref(), Some("Total Sales"));
    assert_eq!(card.sort[0].direction, SortDirection::Descending);
    assert_eq!(
        card.sort[0].field,
        FieldExpr::measure("Orders", "Total Sales")
    );
    assert_eq!(card.filters[0].kind, FilterKind::Advanced);
    assert_eq!(
        card.filters[0].field,
        Some(FieldExpr::measure("Orders", "Total Sales"))
    );
    assert_eq!(
        pbi_pbir::objects::bool_property(&card.objects, "border", "show"),
        Some(false)
    );
    assert_eq!(
        pbi_pbir::objects::bool_property(&card.container_objects, "border", "show"),
        Some(true)
    );

    // Slicer, table (display name override, aggregation, combined Where).
    let slicer = main.visual(SLICER).unwrap();
    assert_eq!(slicer.visual_type, "slicer");
    assert_eq!(slicer.projections[0].active, Some(true));
    assert_eq!(
        slicer.projections[0].field,
        FieldExpr::column("Stores", "Region")
    );
    let table = main.visual(TABLE).unwrap();
    assert_eq!(table.visual_type, "tableEx");
    let labels: Vec<_> = table.projections.iter().map(|p| p.label()).collect();
    assert_eq!(
        labels,
        [
            "Store Name",
            "Store Type",
            "Total Sales",
            "Count of Order ID"
        ]
    );
    assert_eq!(
        table.projections[3].field,
        FieldExpr::aggregation(
            FieldExpr::column("Orders", "Order ID"),
            AggregationFunction::CountNonNull
        )
    );
    assert_eq!(
        table.projections[3].field.query_ref().as_deref(),
        Some(table.projections[3].query_ref.as_str())
    );
    assert_eq!(table.sort[0].direction, SortDirection::Descending);
    assert_eq!(
        table.filters[0].condition,
        Some(Condition::And(
            Box::new(Condition::In {
                exprs: vec![FieldExpr::column("Stores", "Channel")],
                values: vec![vec![Literal::text("Retail")]],
            }),
            Box::new(Condition::Not(Box::new(Condition::Comparison {
                kind: ComparisonKind::Equal,
                left: FieldExpr::column("Stores", "City"),
                right: FieldExpr::Literal(Literal::null()),
            }))),
        ))
    );
    assert!(!table.filters[0].is_hidden_in_view_mode);
    assert!(table.extra.contains_key("query"));

    let textbox = main.visual(TEXTBOX).unwrap();
    assert_eq!(textbox.visual_type, "textbox");
    assert!(textbox.projections.is_empty());
    assert!(textbox.objects["general"][0]["properties"]["paragraphs"].is_array());

    let hidden = detail.visual("3b28913dac9013392389").unwrap();
    assert!(hidden.is_hidden);
    assert_eq!(
        hidden.projections[0].field,
        FieldExpr::measure("Returns", "Returns Count")
    );
}

/// The synthetic layout mirrors part of the Acme Stores PBIR report: the
/// same content in PBIR and legacy form gives the same AST pieces.
#[test]
fn legacy_matches_pbir_for_shared_content() {
    let Some(folder) = common::report_folder("stores") else {
        return;
    };
    let (pbir, _) = to_report_ast(&read_pbir(&common::DirSource(folder)).unwrap());
    let legacy = convert(&read_legacy_layout(LAYOUT.as_bytes()).unwrap());

    let strip = |f: &pbi_model::Filter| {
        (
            f.name.clone(),
            f.field.clone(),
            f.kind.clone(),
            f.condition.clone(),
            f.is_hidden_in_view_mode,
            f.is_locked_in_view_mode,
        )
    };
    assert_eq!(strip(&pbir.filters[0]), strip(&legacy.filters[0]));
    assert_eq!(legacy.pages.len(), 2);
    for lp in &legacy.pages {
        let pp = pbir.page(&lp.name).unwrap();
        assert_eq!(
            (
                &pp.name,
                &pp.display_name,
                pp.width,
                pp.height,
                &pp.display_option,
                pp.is_hidden
            ),
            (
                &lp.name,
                &lp.display_name,
                lp.width,
                lp.height,
                &lp.display_option,
                lp.is_hidden
            )
        );
        for lv in &lp.visuals {
            let pv = pp.visual(&lv.name).unwrap();
            assert_eq!(pv.visual_type, lv.visual_type, "{}", lv.name);
            if lv.name == TABLE {
                assert_eq!(pv.position, lv.position);
                assert_eq!(pv.projections[0].field, lv.projections[0].field);
                assert_eq!(pv.projections[0].query_ref, lv.projections[0].query_ref);
                assert_eq!(pv.sort, lv.sort);
            }
        }
    }
    assert_eq!(pbir.active_page, legacy.active_page);
    assert_eq!(
        pbir.theme.custom_theme.unwrap().name,
        legacy.theme.custom_theme.unwrap().name
    );
}

#[test]
fn invalid_embedded_json_is_a_diagnostic() {
    let mut doc: serde_json::Value = serde_json::from_str(LAYOUT).unwrap();
    doc["sections"][0]["visualContainers"][0]["config"] = "{not json".into();
    doc["filters"] = "[{\"name\": 5}]".into();
    let mut select = doc["sections"][0]["visualContainers"][1]["config"]
        .as_str()
        .unwrap()
        .to_owned();
    select = select.replace(
        "\"queryRef\":\"Stores.Region\"",
        "\"queryRef\":\"Missing.Ref\"",
    );
    doc["sections"][0]["visualContainers"][1]["config"] = select.into();
    let legacy = read_legacy_layout(doc.to_string().as_bytes()).unwrap();
    let codes_seen: Vec<_> = legacy.diagnostics.iter().map(|d| d.code.as_str()).collect();
    assert_eq!(
        codes_seen,
        [codes::INVALID_EMBEDDED_JSON, codes::INVALID_EMBEDDED_JSON]
    );
    let (report, diags) = legacy_to_report_ast(&legacy);
    assert!(report.filters.is_empty());
    assert_eq!(report.pages[0].visuals[0].visual_type, "");
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code, codes::UNRESOLVED_QUERY_REF);
}
