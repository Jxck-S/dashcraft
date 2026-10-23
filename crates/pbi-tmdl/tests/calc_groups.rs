//! Calculation groups: typed mapping (`calculationGroup`, `calculationItem`,
//! selection expressions, format string definitions), canonical writer
//! output and lossless round-trips, and the untyped fallback for shapes the
//! typed model cannot hold.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use pbi_model::{Model, TableKind};
use pbi_tmdl::{clear_origins, load_model, parse_file, write_model};
use pretty_assertions::assert_eq;

/// A calculation-group table in canonical (writer) form.
const TIME_CALC: &str = r#"table 'Time Calc'
	lineageTag: t9

	calculationGroup
		precedence: 20
		multipleOrEmptySelectionExpression = SELECTEDMEASURE()
			formatStringDefinition = "0.0"
		noSelectionExpression = SELECTEDMEASURE() * 1

		/// Current period.
		calculationItem Current = SELECTEDMEASURE()

		calculationItem YTD =
				CALCULATE(
				    SELECTEDMEASURE(),
				    DATESYTD('Date'[Date])
				)
			ordinal: 1

		calculationItem 'YOY %' = DIVIDE(SELECTEDMEASURE(), 2)
			ordinal: 2
			formatStringDefinition = "0.00%"
			state: ready

			annotation Note = x

	column Name
		dataType: string
		sourceColumn: Name
		sortByColumn: Ordinal

	column Ordinal
		dataType: int64
		isHidden
		sourceColumn: Ordinal

	partition 'Time Calc' = calculationGroup
		mode: import
"#;

fn files(text: &str) -> Vec<(String, String)> {
    vec![("tables/Time Calc.tmdl".to_owned(), text.to_owned())]
}

fn normalized(mut model: Model) -> Model {
    clear_origins(&mut model);
    model
}

#[test]
fn calculation_groups_map_to_typed_objects() {
    let loaded = load_model(&files(TIME_CALC)).unwrap();
    assert_eq!(loaded.files[0].to_string(), TIME_CALC);
    let t = loaded.model.table("Time Calc").unwrap();
    assert_eq!(t.kind(), TableKind::CalculationGroup);
    assert!(t.extra.is_empty(), "{:?}", t.extra);
    let g = t.calculation_group.as_ref().unwrap();
    assert_eq!(g.precedence, 20);
    let multi = g.multiple_or_empty_selection_expression.as_ref().unwrap();
    assert_eq!(multi.expression, "SELECTEDMEASURE()");
    assert_eq!(multi.format_string_definition.as_deref(), Some("\"0.0\""));
    assert_eq!(
        g.no_selection_expression.as_ref().unwrap().expression,
        "SELECTEDMEASURE() * 1"
    );
    let names: Vec<_> = g
        .calculation_items
        .iter()
        .map(|i| (i.name.as_str(), i.ordinal))
        .collect();
    assert_eq!(
        names,
        [("Current", None), ("YTD", Some(1)), ("YOY %", Some(2))]
    );
    let current = g.item("current").unwrap();
    assert_eq!(current.description.as_deref(), Some("Current period."));
    let yoy = g.item("YOY %").unwrap();
    assert_eq!(yoy.format_string_definition.as_deref(), Some("\"0.00%\""));
    assert!(yoy.extra.contains_key("state"));
    assert_eq!(yoy.annotations[0].name, "Note");
    assert_eq!(
        g.item("YTD").unwrap().expression,
        "CALCULATE(\n    SELECTEDMEASURE(),\n    DATESYTD('Date'[Date])\n)"
    );
}

#[test]
fn calculation_groups_are_a_writer_fixpoint_and_round_trip() {
    let loaded = load_model(&files(TIME_CALC)).unwrap();
    let written = write_model(&loaded.model, None);
    let table = written
        .iter()
        .find(|(p, _)| p == "tables/Time Calc.tmdl")
        .unwrap();
    assert_eq!(table.1, TIME_CALC);
    let reloaded = load_model(&written).unwrap();
    assert_eq!(normalized(reloaded.model), normalized(loaded.model));
}

#[test]
fn edited_items_are_written_and_unchanged_files_kept_verbatim() {
    // Non-canonical spacing and a comment survive while nothing changes.
    let original = TIME_CALC.replace(
        "\t\tcalculationItem Current",
        "\t\t// keep\n\t\tcalculationItem Current",
    );
    let mut loaded = load_model(&files(&original)).unwrap();
    assert_eq!(loaded.write()[1].1, original);
    let g = loaded
        .model
        .table_mut("Time Calc")
        .unwrap()
        .calculation_group
        .as_mut()
        .unwrap();
    g.calculation_items[0].expression = "SELECTEDMEASURE() + 0".into();
    g.calculation_items[0].format_string_definition = Some("SELECTEDMEASUREFORMATSTRING()".into());
    g.precedence = 0;
    let written = loaded.write();
    let text = &written
        .iter()
        .find(|(p, _)| p == "tables/Time Calc.tmdl")
        .unwrap()
        .1;
    assert!(text.contains(
        "\t\tcalculationItem Current = SELECTEDMEASURE() + 0\n\t\t\tformatStringDefinition = SELECTEDMEASUREFORMATSTRING()\n"
    ));
    assert!(!text.contains("precedence"));
    let reloaded = load_model(&written).unwrap();
    assert_eq!(normalized(reloaded.model), normalized(loaded.model));
}

#[test]
fn untypeable_calculation_groups_stay_untyped() {
    // A named `calculationGroup` and a second declaration cannot be typed:
    // they stay in `extra` (the first typeable one is typed), the table is
    // still a calculation group, and everything round-trips.
    let text = "table G\n\tcalculationGroup Named\n\t\tprecedence: 3\n\n\tcalculationGroup\n\t\tprecedence: x\n\n\t\tcalculationItem A\n\n\tcalculationGroup\n";
    let loaded = load_model(&files(text)).unwrap();
    assert_eq!(parse_file("g.tmdl", text).unwrap().to_string(), text);
    let t = loaded.model.table("G").unwrap();
    assert_eq!(t.kind(), TableKind::CalculationGroup);
    let g = t.calculation_group.as_ref().unwrap();
    assert_eq!(g.precedence, 0);
    assert!(g.extra.contains_key("precedence"));
    assert_eq!(g.calculation_items[0].name, "A");
    assert_eq!(g.calculation_items[0].expression, "");
    assert!(t.extra.contains_key("calculationGroup"));
    let written = write_model(&loaded.model, None);
    let reloaded = load_model(&written).unwrap();
    assert_eq!(normalized(reloaded.model), normalized(loaded.model));
}
