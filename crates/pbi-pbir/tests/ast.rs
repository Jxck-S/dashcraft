#![allow(clippy::unwrap_used, clippy::expect_used)]
//! PBIR → report AST conversion on the Acme Outfitters example reports.

mod common;

use common::{DirSource, memory_copy, report_folder, report_folders};
use pbi_model::{
    AggregationFunction, Condition, DatasetReference, DisplayOption, FieldExpr, FilterKind,
    InteractionKind, Literal, Report, SortDirection, VisualKind,
};
use pbi_pbir::{FileSource, PbirReport, read_pbir, resolve_color, to_report_ast};
use serde_json::json;

const PERFORMANCE: &str = "ef3ce6b6fadf22960137";
const DETAIL: &str = "50c645140e8e10eea677";
const TOOLTIP: &str = "58db5b3f75d059fec1fb";
const ONLINE_OVERVIEW: &str = "a60af4d6244c3a6206a5";
const PIE: &str = "bde8debb2da31c9b193a";

fn convert(label: &str, source: &impl FileSource) -> (PbirReport, Report) {
    let pbir = read_pbir(source).unwrap();
    let (report, diags) = to_report_ast(&pbir);
    assert!(diags.is_empty(), "{label}: {diags:?}");
    (pbir, report)
}

fn load(label: &str) -> Option<(PbirReport, Report)> {
    let folder = report_folder(label)?;
    Some(convert(label, &DirSource(folder)))
}

/// `(page name, display name, width, height, hidden, visuals)`.
type PageFacts = (&'static str, &'static str, f64, f64, bool, usize);

#[test]
fn pages_are_in_page_order_with_sizes_and_visual_counts() {
    let expected: [(&str, &[PageFacts]); 3] = [
        (
            "overview",
            &[
                (
                    "ef796b39c816a4afa4f1",
                    "Executive Summary",
                    1280.0,
                    720.0,
                    false,
                    14,
                ),
                (
                    "59126dbf5023ae95f4dc",
                    "Product Detail",
                    1280.0,
                    1080.0,
                    false,
                    13,
                ),
                (
                    "de11804d145ce7e78111",
                    "Time Intelligence",
                    1280.0,
                    720.0,
                    false,
                    12,
                ),
            ],
        ),
        (
            "stores",
            &[
                (PERFORMANCE, "Store Performance", 1280.0, 720.0, false, 14),
                (DETAIL, "Store Detail", 1280.0, 900.0, true, 5),
                (TOOLTIP, "Store Tooltip", 320.0, 240.0, true, 1),
            ],
        ),
        (
            "online",
            &[
                (ONLINE_OVERVIEW, "Online Overview", 1280.0, 720.0, false, 14),
                (
                    "95c48410a9ac03ae2a38",
                    "Customer Detail",
                    1280.0,
                    960.0,
                    false,
                    7,
                ),
            ],
        ),
    ];
    for (label, pages) in expected {
        let Some((_, report)) = load(label) else {
            continue;
        };
        let actual: Vec<PageFacts> = report
            .pages
            .iter()
            .map(|p| {
                let name: &'static str = pages.iter().find(|e| e.0 == p.name).map_or("?", |e| e.0);
                let display: &'static str = pages
                    .iter()
                    .find(|e| e.1 == p.display_name)
                    .map_or("?", |e| e.1);
                (
                    name,
                    display,
                    p.width,
                    p.height,
                    p.is_hidden,
                    p.visuals.len(),
                )
            })
            .collect();
        assert_eq!(actual, pages, "{label}");
        for page in &report.pages {
            assert!(page.origin.as_ref().unwrap().path.ends_with("page.json"));
        }
        assert_eq!(report.active_page.as_deref(), Some(pages[0].0), "{label}");
        assert_eq!(report.start_page().unwrap().name, pages[0].0);
        assert!(report.theme.base_theme.is_none());
        let custom = report.theme.custom_theme.as_ref().unwrap();
        assert_eq!(custom.name, "AcmeOutfitters.json");
        assert_eq!(custom.kind.as_deref(), Some("RegisteredResources"));
        assert_eq!(report.schema_versions["report"], "3.3.0");
    }
}

#[test]
fn stores_report_details() {
    let Some((pbir, report)) = load("stores") else {
        return;
    };
    assert_eq!(report.name, "Acme Outfitters - Stores");
    assert_eq!(
        report.dataset_reference,
        DatasetReference::ByPath {
            path: "../AcmeOutfitters.SemanticModel".into()
        }
    );
    assert_eq!(report.schema_versions["definition.pbir"], "4.0");
    let main = report.page(PERFORMANCE).unwrap();
    assert!(main.schema.as_deref().unwrap().contains("/page/2.1.0/"));
    assert_eq!(main.display_option, DisplayOption::FitToPage);
    assert_eq!(
        report.page(DETAIL).unwrap().display_option,
        DisplayOption::FitToWidth
    );
    assert_eq!(
        report.page(TOOLTIP).unwrap().display_option,
        DisplayOption::ActualSize
    );
    let mut types: Vec<_> = report
        .visuals()
        .map(|(_, v)| v.visual_type.as_str())
        .collect();
    types.sort_unstable();
    types.dedup();
    assert_eq!(
        types,
        [
            "actionButton",
            "cardVisual",
            "clusteredColumnChart",
            "gauge",
            "hundredPercentStackedBarChart",
            "lineClusteredColumnComboChart",
            "map",
            "multiRowCard",
            "shape",
            "slicer",
            "stackedColumnChart",
            "tableEx",
            "textbox",
            "waterfallChart"
        ]
    );
    assert!(report.visuals().all(|(_, v)| v.kind == VisualKind::Visual));

    // Report-level filter: Stores[Channel] IN {'Retail', 'Outlet'}.
    assert_eq!(report.filters.len(), 1);
    let filter = &report.filters[0];
    assert_eq!(filter.name, "b0d63946d6634cc987ce");
    assert_eq!(filter.kind, FilterKind::Categorical);
    assert!(!filter.is_hidden_in_view_mode && !filter.is_locked_in_view_mode);
    assert_eq!(filter.field, Some(FieldExpr::column("Stores", "Channel")));
    assert_eq!(
        filter.condition,
        Some(Condition::In {
            exprs: vec![FieldExpr::column("Stores", "Channel")],
            values: vec![vec![Literal::text("Retail")], vec![Literal::text("Outlet")]],
        })
    );
    let origin = filter.origin.as_ref().unwrap();
    assert_eq!(origin.path, "definition/report.json");
    assert_eq!(origin.pointer.as_deref(), Some("/filterConfig/filters/0"));
    let original: serde_json::Value =
        serde_json::from_slice(&pbir.files["definition/report.json"].bytes).unwrap();
    assert_eq!(filter.raw, original["filterConfig"]["filters"][0]);

    // Page filter of the hidden detail page.
    let detail = report.page(DETAIL).unwrap();
    assert_eq!(
        detail.filters[0].condition,
        Some(Condition::In {
            exprs: vec![FieldExpr::column("Stores", "Region")],
            values: vec![vec![Literal::text("East")]],
        })
    );

    // New card with five measure projections.
    let card = main.visual("c81376feb163c3047446").unwrap();
    assert_eq!(card.visual_type, "cardVisual");
    assert_eq!(card.position.x, 280.0);
    assert_eq!((card.position.width, card.position.height), (976.0, 136.0));
    assert_eq!(card.position.tab_order, Some(6000));
    assert_eq!(card.projections.len(), 5);
    let p = &card.projections[0];
    assert_eq!(p.role, "Data");
    assert_eq!(p.field, FieldExpr::measure("Orders", "Total Sales"));
    assert_eq!(p.query_ref, "Orders.Total Sales");
    assert_eq!(p.label(), "Total Sales");
    assert_eq!(
        card.projections[2].field,
        FieldExpr::measure("Targets", "Target Attainment %")
    );
    assert!(
        card.origin
            .as_ref()
            .unwrap()
            .path
            .ends_with("pages/ef3ce6b6fadf22960137/visuals/c81376feb163c3047446/visual.json")
    );
    assert_eq!(
        pbi_pbir::objects::bool_property(&card.objects, "outline", "show"),
        Some(false)
    );
    assert_eq!(
        pbi_pbir::objects::bool_property(&card.container_objects, "border", "show"),
        Some(true)
    );

    // Table sorted by a measure.
    let table = main.visual("d4431298c2010cc0f494").unwrap();
    assert_eq!(table.sort.len(), 1);
    assert_eq!(table.sort[0].direction, SortDirection::Descending);
    assert_eq!(
        table.sort[0].field,
        FieldExpr::measure("Orders", "Total Sales")
    );

    // The header shape and its literal fill color.
    let header = main.visual("9b2792b8239b52dc5cbc").unwrap();
    assert_eq!(header.position.z, 0.0);
    assert_eq!(header.position.width, 1280.0);
    let fill = pbi_pbir::objects::property(&header.objects, "fill", "fillColor").unwrap();
    assert_eq!(
        resolve_color(fill, pbir.custom_theme()).as_deref(),
        Some("#1F3A2E")
    );
}

#[test]
fn visual_filters_and_top_n() {
    let Some((_, report)) = load("overview") else {
        return;
    };
    let table = report
        .page("59126dbf5023ae95f4dc")
        .unwrap()
        .visual("fc4760750cac58a5aa34")
        .unwrap();
    assert_eq!(table.visual_type, "tableEx");
    assert_eq!(table.sort[0].direction, SortDirection::Descending);
    let top_n = &table.filters[0];
    assert_eq!(top_n.kind, FilterKind::TopN);
    assert_eq!(
        top_n.field,
        Some(FieldExpr::column("Products", "Product Name"))
    );
    assert_eq!(top_n.how_created.as_deref(), Some("User"));
    // Report filter: Orders[Status] NOT IN {'Cancelled'}.
    assert_eq!(
        report.filters[0].condition,
        Some(Condition::Not(Box::new(Condition::In {
            exprs: vec![FieldExpr::column("Orders", "Status")],
            values: vec![vec![Literal::text("Cancelled")]],
        })))
    );
}

#[test]
fn custom_theme_colors_resolve() {
    let Some((pbir, _)) = load("stores") else {
        return;
    };
    let theme = pbir.custom_theme().expect("custom theme");
    assert!(pbir.base_theme().is_none());
    assert_eq!(theme.name.as_deref(), Some("Acme Outfitters"));
    assert_eq!(theme.data_colors.as_ref().unwrap()[0], "#2F6B4F");
    assert_eq!(theme.foreground.as_deref(), Some("#252423"));
    let callout = theme.text_class("callout").unwrap();
    assert_eq!(callout.font_size_f64(), Some(24.0));
    assert_eq!(callout.font_face.as_deref(), Some("Segoe UI Semibold"));
    assert!(theme.visual_styles.as_ref().unwrap()["*"]["*"].is_object());
    // ColorId 2 is the first data color; Percent lightens towards white.
    assert_eq!(
        pbi_pbir::resolve_theme_data_color(theme, 2, 0.0).as_deref(),
        Some("#2F6B4F")
    );
    assert_eq!(
        pbi_pbir::resolve_theme_data_color(theme, 3, 0.4).as_deref(),
        Some("#E8B480")
    );
    let color =
        json!({"solid": {"color": {"expr": {"ThemeDataColor": {"ColorId": 3, "Percent": 0.4}}}}});
    assert_eq!(
        resolve_color(&color, Some(theme)).as_deref(),
        Some("#E8B480")
    );
}

#[test]
fn visual_interactions_are_parsed() {
    let Some((_, report)) = load("stores") else {
        return;
    };
    let page = report.page(PERFORMANCE).unwrap();
    assert_eq!(page.visual_interactions.len(), 4);
    assert_eq!(
        page.interaction("05ac8028fb6e14374ec7", "d4431298c2010cc0f494"),
        Some(&InteractionKind::DataFilter)
    );
    assert_eq!(
        page.interaction("05ac8028fb6e14374ec7", "8ee1ff74df542ac157fc"),
        Some(&InteractionKind::NoFilter)
    );
    assert_eq!(
        page.interaction("69f47593f5e0a4aefd36", "8ee1ff74df542ac157fc"),
        Some(&InteractionKind::HighlightFilter)
    );
    assert_eq!(
        page.interaction("8ee1ff74df542ac157fc", "05ac8028fb6e14374ec7"),
        None
    );
}

#[test]
fn online_report_filters_on_two_tables() {
    let Some((_, report)) = load("online") else {
        return;
    };
    let conditions: Vec<_> = report.filters.iter().map(|f| f.condition.clone()).collect();
    assert_eq!(
        conditions,
        [
            Some(Condition::In {
                exprs: vec![FieldExpr::column("Stores", "Channel")],
                values: vec![vec![Literal::text("Online")]],
            }),
            Some(Condition::In {
                exprs: vec![FieldExpr::column("Web Sessions", "Device")],
                values: vec![vec![Literal::text("Mobile")]],
            }),
        ]
    );
}

/// An implicit aggregation and a theme color in a visual (the example's
/// visuals use measures and literal colors, so the pie chart is edited).
#[test]
fn implicit_aggregation_and_theme_colors_in_a_visual() {
    let Some(folder) = report_folder("online") else {
        return;
    };
    let path = format!("definition/pages/{ONLINE_OVERVIEW}/visuals/{PIE}/visual.json");
    let mut visual: serde_json::Value =
        serde_json::from_slice(&std::fs::read(folder.join(&path)).unwrap()).unwrap();
    visual["visual"]["query"]["queryState"]["Y"]["projections"] = json!([{
        "field": {"Aggregation": {
            "Expression": {"Column": {"Expression": {"SourceRef": {"Entity": "Web Sessions"}}, "Property": "Session ID"}},
            "Function": 5
        }},
        "queryRef": "CountNonNull(Web Sessions.Session ID)",
        "nativeQueryRef": "Count of Session ID"
    }]);
    visual["visual"]["objects"]["dataPoint"] = json!([{"properties": {"fill": {"solid": {"color": {"expr": {"ThemeDataColor": {"ColorId": 2, "Percent": 0}}}}}}}]);
    let mut source = memory_copy(&folder);
    source.insert(path, serde_json::to_vec_pretty(&visual).unwrap());
    let (pbir, report) = convert("online (edited)", &source);
    let pie = report.page(ONLINE_OVERVIEW).unwrap().visual(PIE).unwrap();
    let y = pie.projections_for("Y").next().unwrap();
    assert_eq!(
        y.field,
        FieldExpr::aggregation(
            FieldExpr::column("Web Sessions", "Session ID"),
            AggregationFunction::CountNonNull
        )
    );
    assert_eq!(y.field.query_ref().unwrap(), y.query_ref);
    assert_eq!(y.label(), "Count of Session ID");
    assert_eq!(pie.roles(), ["Category", "Y"]);
    let fill = pbi_pbir::objects::property(&pie.objects, "dataPoint", "fill").unwrap();
    assert_eq!(
        resolve_color(fill, pbir.custom_theme()).as_deref(),
        Some("#2F6B4F")
    );
}

/// A report bound to a remote semantic model (`byConnection`), and a report
/// folder as stored inside a PBIX (no `definition.pbir`, no `.platform`).
#[test]
fn remote_and_packaged_report_folders() {
    let Some(folder) = report_folder("overview") else {
        return;
    };
    let remote = json!({
        "$schema": "https://developer.microsoft.com/json-schemas/fabric/item/report/definitionProperties/2.0.0/schema.json",
        "version": "4.0",
        "datasetReference": {"byConnection": {
            "connectionString": "Data Source=\"powerbi://api.powerbi.com/v1.0/myorg/Acme Outfitters\";initial catalog=AcmeOutfitters;integrated security=ClaimsToken;semanticmodelid=00000000-0000-4000-8000-0000000000ac"
        }}
    });
    let mut source = memory_copy(&folder);
    source.insert(
        "definition.pbir",
        serde_json::to_vec_pretty(&remote).unwrap(),
    );
    let (pbir, report) = convert("remote", &source);
    let DatasetReference::ByConnection {
        connection_string, ..
    } = &report.dataset_reference
    else {
        panic!("expected byConnection");
    };
    assert!(
        connection_string
            .as_deref()
            .unwrap()
            .contains("semanticmodelid=")
    );
    assert!(report.dataset_reference.is_remote());
    assert_eq!(report.schema_versions["definitionProperties"], "2.0.0");
    assert_eq!(report.pages.len(), 3);
    let written = pbi_pbir::write_pbir(&pbir);
    let (_, bytes) = written
        .iter()
        .find(|(p, _)| p == "definition.pbir")
        .unwrap();
    assert_eq!(bytes, &serde_json::to_vec_pretty(&remote).unwrap());

    let mut packaged = memory_copy(&folder);
    packaged.files.remove("definition.pbir");
    packaged.files.remove(".platform");
    packaged.insert("LinguisticSchema", b"<LinguisticSchema/>".to_vec());
    let (pbir, report) = convert("packaged", &packaged);
    assert!(pbir.definition_pbir.is_none() && pbir.platform.is_none());
    assert_eq!(report.dataset_reference, DatasetReference::None);
    assert!(pbir.files.contains_key("LinguisticSchema"));
    assert_eq!(report.pages.len(), 3);
}

/// Every column, measure and aggregation reference resolves. Relative date
/// values (`DateSpan`/`DateAdd`/`Now` on the right of a comparison) are not
/// field references; they are kept verbatim as `FieldExpr::Unknown`.
#[test]
fn every_field_reference_is_resolved() {
    let is_date_value = |v: &serde_json::Value| {
        ["DateSpan", "DateAdd", "Now"]
            .iter()
            .any(|k| v.get(k).is_some())
    };
    let mut date_values = 0;
    for (label, folder) in report_folders() {
        let pbir = read_pbir(&DirSource(folder)).unwrap();
        let (report, _) = to_report_ast(&pbir);
        let mut unknown = Vec::new();
        report.visit_fields(&mut |f| match f.base() {
            FieldExpr::Unknown(v) if is_date_value(v) => date_values += 1,
            FieldExpr::Unknown(_) => unknown.push(f.clone()),
            _ => {}
        });
        assert!(unknown.is_empty(), "{label}: {unknown:?}");
    }
    if report_folder("overview").is_some() {
        // The relative date filter of the Time Intelligence line chart.
        assert_eq!(date_values, 2);
    }
}
