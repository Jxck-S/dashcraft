//! Shared test fixture: a small hand-built document in the style of the Acme
//! Outfitters example (`reference/examples/acme-outfitters`).

use serde_json::json;

use crate::document::{Document, ModelStatus, SourceFormat, SourceInfo};
use crate::props::{Annotation, Origin, PropValue};
use crate::query::{
    AggregationFunction, ComparisonKind, Condition, FieldExpr, Filter, FilterKind, Literal,
    SortDirection, SortItem,
};
use crate::report::{
    DatasetReference, DisplayOption, InteractionKind, Page, Position, Projection, Report,
    ResourceItem, ResourcePackage, ResourceRef, ThemeRef, Visual, VisualInteraction,
};
use crate::semantic::{
    Column, ColumnKind, ColumnRef, CrossFilter, DataType, Database, Hierarchy, Level, Measure,
    Model, NamedExpression, Partition, PartitionMode, PartitionSource, Relationship, Role,
    SummarizeBy, Table, TablePermission,
};

fn m_partition(name: &str, expression: &str) -> Partition {
    Partition {
        name: name.into(),
        mode: PartitionMode::Import,
        source: PartitionSource::M {
            expression: expression.into(),
        },
        ..Partition::default()
    }
}

fn projection(role: &str, field: FieldExpr, native: &str) -> Projection {
    Projection {
        role: role.into(),
        query_ref: field.query_ref().unwrap_or_default(),
        field,
        native_query_ref: Some(native.into()),
        ..Projection::default()
    }
}

fn model() -> Model {
    let mut stores = Table::new("Stores");
    stores.description = Some("Stores and online channels.".into());
    stores.lineage_tag = Some("a0c1e2f3-0000-4000-8000-000000000001".into());
    let mut key = Column::data("Store ID", DataType::String);
    key.is_hidden = true;
    key.summarize_by = Some(SummarizeBy::None);
    let mut region = Column::data("Region", DataType::String);
    region.sort_by_column = Some("Region Rank".into());
    region.description = Some("Sales region of the store.".into());
    region
        .extra
        .insert("isAvailableInMdx".into(), PropValue::Text("false".into()));
    stores.columns = vec![
        key,
        Column::data("Store Name", DataType::String),
        region,
        Column::data("Region Rank", DataType::Int64),
        Column::calculated(
            "Sales Status",
            DataType::String,
            "IF(ISEMPTY(RELATEDTABLE(Orders)), \"No Sales\", \"Has Sales\")",
        ),
    ];
    let mut total = Measure::new("Store Count", "DISTINCTCOUNTNOBLANK(Stores[Store Name])");
    total.format_string = Some("0".into());
    total.description = Some("Distinct count of stores.".into());
    stores.measures = vec![
        total,
        Measure::new(
            "Active Stores",
            "CALCULATE(COUNTROWS(Stores), Stores[Sales Status] = \"Has Sales\") + 0",
        ),
    ];
    stores.hierarchies = vec![Hierarchy {
        name: "Geography".into(),
        levels: vec![
            Level {
                name: "Region".into(),
                ordinal: Some(0),
                column: "Region".into(),
                ..Level::default()
            },
            Level {
                name: "Store".into(),
                ordinal: Some(1),
                column: "Store Name".into(),
                ..Level::default()
            },
        ],
        ..Hierarchy::default()
    }];
    stores.partitions = vec![m_partition(
        "Stores",
        "let\n    Source = AzureStorage.Blobs(StorageContainerUrl)\nin\n    Source",
    )];
    stores.annotations = vec![Annotation::new("PBI_ResultType", "Table")];
    stores.origin = Some(Origin::file("definition/tables/Stores.tmdl").with_line(2));

    let mut orders = Table::new("Orders");
    orders.columns = vec![
        Column::data("Store ID", DataType::String),
        Column::data("Order ID", DataType::String),
    ];
    orders.partitions = vec![m_partition("Orders", "let Source = 1 in Source")];

    let mut unknown = Table::new("Unknown Store Orders");
    unknown.columns = vec![Column {
        name: "Order ID".into(),
        data_type: DataType::String,
        is_name_inferred: true,
        kind: ColumnKind::CalculatedTableColumn {
            source_column: "[Order ID]".into(),
        },
        ..Column::default()
    }];
    unknown.partitions = vec![Partition {
        name: "Unknown Store Orders".into(),
        mode: PartitionMode::Import,
        source: PartitionSource::Calculated {
            expression: "SELECTCOLUMNS(FILTER(...), \"Order ID\", Orders[Order ID])".into(),
        },
        ..Partition::default()
    }];

    let mut relationship = Relationship::new(
        "d0c1e2f3-a4b5-4c6d-8e7f-000000000001",
        ColumnRef::new("Orders", "Store ID"),
        ColumnRef::new("Stores", "Store ID"),
    );
    relationship.cross_filtering_behavior = CrossFilter::BothDirections;

    let mut blob = NamedExpression::m(
        "StorageContainerUrl",
        r#""https://acmeoutfittersdemo.blob.core.windows.net/sample-data" meta [IsParameterQuery=true, Type="Text", IsParameterQueryRequired=true]"#,
    );
    blob.description = Some("Azure Blob container holding the source CSVs.".into());

    Model {
        name: "Model".into(),
        culture: Some("en-US".into()),
        default_power_bi_data_source_version: Some("powerBI_V3".into()),
        source_query_culture: Some("en-US".into()),
        database: Some(Database {
            compatibility_level: Some(1601),
            ..Database::default()
        }),
        tables: vec![stores, orders, unknown],
        relationships: vec![relationship],
        expressions: vec![blob],
        roles: vec![Role {
            name: "Region West".into(),
            model_permission: Some("read".into()),
            table_permissions: vec![TablePermission {
                table: "Stores".into(),
                filter_expression: Some("[Region] = \"West\"".into()),
                ..TablePermission::default()
            }],
            ..Role::default()
        }],
        ..Model::default()
    }
}

fn region_filter(name: &str) -> Filter {
    Filter {
        name: name.into(),
        field: Some(FieldExpr::column("Stores", "Region")),
        kind: FilterKind::Categorical,
        condition: Some(Condition::In {
            exprs: vec![FieldExpr::column("Stores", "Region")],
            values: vec![vec![Literal::text("West")]],
        }),
        is_hidden_in_view_mode: true,
        is_locked_in_view_mode: true,
        raw: json!({
            "name": name,
            "field": {"Column": {"Expression": {"SourceRef": {"Entity": "Stores"}}, "Property": "Region"}},
            "type": "Categorical",
            "filter": {
                "Version": 2,
                "From": [{"Name": "s", "Entity": "Stores", "Type": 0}],
                "Where": [{"Condition": {"In": {
                    "Expressions": [{"Column": {"Expression": {"SourceRef": {"Source": "s"}}, "Property": "Region"}}],
                    "Values": [[{"Literal": {"Value": "'West'"}}]]
                }}}]
            },
            "isHiddenInViewMode": true,
            "isLockedInViewMode": true
        }),
        origin: Some(
            Origin::file("definition/report.json").with_pointer("/filterConfig/filters/0"),
        ),
        ..Filter::default()
    }
}

fn report() -> Report {
    let slicer = Visual {
        name: "5a1c3e7b9d2f4a6c8e0b".into(),
        visual_type: "slicer".into(),
        position: Position {
            x: 16.0,
            y: 480.0,
            z: 15000.0,
            width: 220.0,
            height: 60.0,
            tab_order: Some(15000),
            angle: None,
        },
        projections: vec![Projection {
            active: Some(true),
            ..projection(
                "Values",
                FieldExpr::column("Stores", "Store Name"),
                "Store Name",
            )
        }],
        objects: json!({"general": [{"properties": {"outlineColor": {"solid": {"color": {"expr": {"ThemeDataColor": {"ColorId": 1, "Percent": 0}}}}}}}]}),
        container_objects: json!({"border": [{"properties": {"show": {"expr": {"Literal": {"Value": "true"}}}}}]}),
        ..Visual::default()
    };
    let card = Visual {
        name: "card1".into(),
        visual_type: "cardVisual".into(),
        position: Position {
            x: 240.0,
            y: 10.0,
            z: 1000.0,
            width: 200.0,
            height: 100.0,
            ..Position::default()
        },
        projections: vec![projection(
            "Data",
            FieldExpr::measure("Stores", "Active Stores"),
            "Active Stores",
        )],
        ..Visual::default()
    };
    let count = FieldExpr::aggregation(
        FieldExpr::column("Stores", "Store Name"),
        AggregationFunction::CountNonNull,
    );
    let bar = Visual {
        name: "bar1".into(),
        visual_type: "clusteredBarChart".into(),
        position: Position {
            x: 460.0,
            y: 10.0,
            z: 2000.0,
            width: 400.0,
            height: 300.0,
            ..Position::default()
        },
        projections: vec![
            projection("Category", FieldExpr::column("Stores", "Region"), "Region"),
            projection("Y", count.clone(), "Count of Store Name"),
        ],
        sort: vec![SortItem {
            field: count,
            direction: SortDirection::Descending,
        }],
        filters: vec![Filter {
            name: "f-visual".into(),
            field: Some(FieldExpr::column("Stores", "Region")),
            ..Filter::default()
        }],
        objects: json!({"dataPoint": [{"properties": {"fill": {"solid": {"color": {"expr": {"Literal": {"Value": "'#2E7D32'"}}}}}}, "selector": {"metadata": "Stores.Region"}}]}),
        ..Visual::default()
    };
    let main = Page {
        name: "8b2d4f6a1c3e5a7b9d0f".into(),
        display_name: "Store Performance".into(),
        display_option: DisplayOption::FitToWidth,
        filters: vec![Filter {
            name: "f-page".into(),
            field: Some(FieldExpr::column("Orders", "Order ID")),
            kind: FilterKind::Advanced,
            condition: Some(Condition::Not(Box::new(Condition::Comparison {
                kind: ComparisonKind::Equal,
                left: FieldExpr::column("Orders", "Order ID"),
                right: FieldExpr::Literal(Literal::null()),
            }))),
            ..Filter::default()
        }],
        visuals: vec![slicer, card, bar],
        visual_interactions: vec![VisualInteraction {
            source: "5a1c3e7b9d2f4a6c8e0b".into(),
            target: "bar1".into(),
            kind: InteractionKind::DataFilter,
        }],
        objects: json!({"outspacePane": [{"properties": {"width": {"expr": {"Literal": {"Value": "240L"}}}}}]}),
        ..Page::default()
    };
    let unknown = Page {
        name: "7c3e5a9b1d2f4e6a8c0b".into(),
        display_name: "Unknown Store Orders".into(),
        visuals: vec![Visual {
            name: "4e6a8c0b2d4f6a1c3e5b".into(),
            visual_type: "tableEx".into(),
            projections: vec![projection(
                "Values",
                FieldExpr::column("Unknown Store Orders", "Order ID"),
                "Order ID",
            )],
            ..Visual::default()
        }],
        ..Page::default()
    };
    Report {
        name: "AcmeOutfitters".into(),
        schema: Some(
            "https://developer.microsoft.com/json-schemas/fabric/item/report/definition/report/3.3.0/schema.json"
                .into(),
        ),
        theme: ThemeRef {
            base_theme: Some(ResourceRef {
                name: "CY24SU10".into(),
                kind: Some("SharedResources".into()),
                ..ResourceRef::default()
            }),
            custom_theme: None,
        },
        filters: vec![region_filter("regionWest")],
        pages: vec![main, unknown],
        active_page: Some("8b2d4f6a1c3e5a7b9d0f".into()),
        settings: json!({"useStylableVisualContainerHeader": true, "exportDataMode": "AllowSummarized"}),
        dataset_reference: DatasetReference::ByConnection {
            connection_string: Some(
                "Data Source=powerbi://api.powerbi.com/v1.0/myorg/Acme Outfitters".into(),
            ),
            model_id: None,
            database_name: Some("00000000-0000-4000-8000-0000000000ac".into()),
            connection_type: Some("pbiServiceXmlaStyleLive".into()),
            name: Some("EntityDataSource".into()),
            raw: json!({"pbiServiceModelId": null, "timeoutSeconds": 30, "ratio": 1.5}),
        },
        resource_packages: vec![ResourcePackage {
            name: "SharedResources".into(),
            kind: Some("SharedResources".into()),
            items: vec![ResourceItem {
                name: "CY24SU10".into(),
                path: "BaseThemes/CY24SU10.json".into(),
                kind: Some("BaseTheme".into()),
                ..ResourceItem::default()
            }],
            ..ResourcePackage::default()
        }],
        schema_versions: [("report".to_owned(), "3.3.0".to_owned())]
            .into_iter()
            .collect(),
        ..Report::default()
    }
}

/// A fully ID-assigned document with a model and one report.
pub(crate) fn sample_document() -> Document {
    let mut doc = Document::new(SourceInfo {
        format: SourceFormat::Pbip,
        path: "AcmeOutfitters.pbip".into(),
        entries: vec!["AcmeOutfitters.pbip".into()],
    });
    doc.model = Some(model());
    doc.model_status = ModelStatus::Local;
    doc.reports.push(report());
    doc.assign_ids();
    doc
}
