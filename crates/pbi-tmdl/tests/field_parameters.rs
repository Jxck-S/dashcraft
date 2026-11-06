//! Field parameters: a display column grouped by `Parameter Fields`
//! (`relatedColumnDetails` / `groupByColumn`) is read, written in canonical
//! form, and written from the TOM JSON shape a TMSL reader keeps.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use pbi_model::{Column, DataType, Model, Table};
use pbi_tmdl::{load_model, write_model};
use pretty_assertions::assert_eq;

/// A field-parameter table in canonical (writer) form.
const TOGGLE: &str = r#"table 'Toggle for breakdown'
	lineageTag: t1

	column 'Breakdown by'
		dataType: string
		lineageTag: c1
		summarizeBy: none
		sourceColumn: [Value1]
		sortByColumn: 'Parameter Order'
		relatedColumnDetails
			groupByColumn: 'Parameter Fields'

	column 'Parameter Fields'
		dataType: string
		isHidden
		lineageTag: c2
		summarizeBy: none
		sourceColumn: [Value2]

	column 'Parameter Order'
		dataType: int64
		isHidden
		lineageTag: c3
		summarizeBy: sum
		sourceColumn: [Value3]

	partition 'Toggle for breakdown' = calculated
		mode: import
		source = {("Region", NAMEOF('Geo'[Region]), 0)}
"#;

#[test]
fn group_by_columns_are_read_and_written_canonically() {
    let files = vec![(
        "tables/Toggle for breakdown.tmdl".to_owned(),
        TOGGLE.to_owned(),
    )];
    let loaded = load_model(&files).unwrap();
    let t = loaded.model.table("Toggle for breakdown").unwrap();
    assert_eq!(
        t.column("Breakdown by").unwrap().group_by_columns(),
        ["Parameter Fields"]
    );
    assert!(
        t.column("Parameter Fields")
            .unwrap()
            .group_by_columns()
            .is_empty()
    );
    let written = write_model(&loaded.model, None);
    let table = written
        .iter()
        .find(|(p, _)| p == "tables/Toggle for breakdown.tmdl")
        .unwrap();
    assert_eq!(table.1, TOGGLE);
}

#[test]
fn tom_json_group_by_columns_are_written_as_tmdl() {
    let mut display = Column::data("# / %", DataType::String);
    display.extra.insert(
        "relatedColumnDetails".into(),
        serde_json::json!({"groupByColumns": [{"groupingColumn": "Parameter Fields"}]}).into(),
    );
    let mut table = Table::new("Toggle");
    table.columns = vec![display, Column::data("Parameter Fields", DataType::String)];
    let model = Model {
        tables: vec![table],
        ..Model::default()
    };
    let written = write_model(&model, None);
    let text = &written
        .iter()
        .find(|(p, _)| p == "tables/Toggle.tmdl")
        .unwrap()
        .1;
    assert!(
        text.contains("\t\trelatedColumnDetails\n\t\t\tgroupByColumn: 'Parameter Fields'\n"),
        "{text}"
    );
    let reloaded = load_model(&written).unwrap();
    let column = reloaded
        .model
        .table("Toggle")
        .unwrap()
        .column("# / %")
        .unwrap()
        .group_by_columns();
    assert_eq!(column, ["Parameter Fields"]);
}
