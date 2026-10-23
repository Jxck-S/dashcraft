use pretty_assertions::assert_eq;

use super::*;
use crate::query::{Condition, FieldExpr};
use crate::report::Position;
use crate::test_support::sample_document;

fn table_id(doc: &Document, name: &str) -> ObjectId {
    doc.model.as_ref().unwrap().table(name).unwrap().id
}

fn column_id(doc: &Document, table: &str, column: &str) -> ObjectId {
    doc.model
        .as_ref()
        .unwrap()
        .column(table, column)
        .unwrap()
        .id
}

fn measure_id(doc: &Document, table: &str, measure: &str) -> ObjectId {
    doc.model
        .as_ref()
        .unwrap()
        .measure(table, measure)
        .unwrap()
        .id
}

/// execute → check → undo == original → redo == after.
fn assert_round_trip(
    doc: &mut Document,
    command: impl Command + 'static,
    check: impl FnOnce(&Document, &[ChangeEvent]),
) {
    let original = doc.clone();
    let mut history = History::new();
    let events = history.execute(doc, command).unwrap();
    check(doc, &events);
    let after = doc.clone();
    assert_ne!(after, original);
    history.undo(doc).unwrap();
    assert_eq!(*doc, original);
    history.redo(doc).unwrap();
    assert_eq!(*doc, after);
    history.undo(doc).unwrap();
    assert_eq!(*doc, original);
}

#[test]
fn rename_table_rewrites_references_and_undoes_exactly() {
    let mut doc = sample_document();
    let id = table_id(&doc, "Stores");
    let rel_id = doc.model.as_ref().unwrap().relationships[0].id;
    let command = RenameTable {
        table: id,
        new_name: "Shops".into(),
    };
    assert_round_trip(&mut doc, command, |doc, events| {
        let model = doc.model.as_ref().unwrap();
        assert!(model.table("Stores").is_none());
        assert_eq!(model.table_by_id(id).unwrap().name, "Shops");
        assert_eq!(model.relationships[0].to.table, "Shops");
        assert_eq!(model.relationships[0].from.table, "Orders");
        assert_eq!(model.roles[0].table_permissions[0].table, "Shops");

        let report = &doc.reports[0];
        let filter = &report.filters[0];
        assert_eq!(filter.field.as_ref().unwrap().entity(), Some("Shops"));
        let Some(Condition::In { exprs, .. }) = &filter.condition else {
            panic!("expected In condition");
        };
        assert_eq!(exprs[0].entity(), Some("Shops"));
        assert_eq!(
            filter.raw["field"]["Column"]["Expression"]["SourceRef"]["Entity"],
            "Shops"
        );
        assert_eq!(filter.raw["filter"]["From"][0]["Entity"], "Shops");

        let page = &report.pages[0];
        let slicer = &page.visuals[0];
        assert_eq!(slicer.projections[0].query_ref, "Shops.Store Name");
        assert_eq!(
            slicer.projections[0].native_query_ref.as_deref(),
            Some("Store Name")
        );
        let bar = &page.visuals[2];
        assert_eq!(
            bar.projections[1].query_ref,
            "CountNonNull(Shops.Store Name)"
        );
        assert_eq!(bar.sort[0].field.entity(), Some("Shops"));
        assert_eq!(
            bar.objects["dataPoint"][0]["selector"]["metadata"],
            "Shops.Region"
        );
        // Untouched references stay as they were.
        assert_eq!(
            page.filters[0].field.as_ref().unwrap().entity(),
            Some("Orders")
        );

        assert!(events.contains(&ChangeEvent::ObjectRenamed {
            id,
            old: "Stores".into(),
            new: "Shops".into(),
        }));
        assert!(events.contains(&ChangeEvent::ObjectChanged { id: rel_id }));
        assert!(events.contains(&ChangeEvent::ObjectChanged { id: bar.id }));
        assert!(events.contains(&ChangeEvent::ObjectChanged { id: report.id }));
        // The unrelated table visual on page 2 is not reported.
        let other = &report.pages[1].visuals[0];
        assert!(!events.iter().any(|e| e.id() == other.id));
    });
}

#[test]
fn rename_column_updates_model_and_report_references() {
    let mut doc = sample_document();
    let original = doc.clone();
    let mut history = History::new();

    let rank = column_id(&doc, "Stores", "Region Rank");
    history
        .execute(
            &mut doc,
            RenameColumn {
                column: rank,
                new_name: "Rank".into(),
            },
        )
        .unwrap();
    let region = column_id(&doc, "Stores", "Region");
    let events = history
        .execute(
            &mut doc,
            RenameColumn {
                column: region,
                new_name: "Area".into(),
            },
        )
        .unwrap();
    let key = column_id(&doc, "Orders", "Store ID");
    history
        .execute(
            &mut doc,
            RenameColumn {
                column: key,
                new_name: "Key".into(),
            },
        )
        .unwrap();

    let model = doc.model.as_ref().unwrap();
    let stores_table = model.table("Stores").unwrap();
    let area = stores_table.column("Area").unwrap();
    assert_eq!(area.id, region);
    assert_eq!(area.sort_by_column.as_deref(), Some("Rank"));
    // The source column is the M output name and must not change.
    assert_eq!(area.source_column(), Some("Region"));
    assert_eq!(stores_table.hierarchies[0].levels[0].column, "Area");
    assert_eq!(model.relationships[0].from.column, "Key");
    assert_eq!(model.relationships[0].to.column, "Store ID");

    let report = &doc.reports[0];
    let filter = &report.filters[0];
    assert_eq!(filter.field.as_ref().unwrap().property(), Some("Area"));
    // Alias-based reference (`SourceRef.Source = "s"`) in the raw JSON.
    assert_eq!(
        filter.raw["filter"]["Where"][0]["Condition"]["In"]["Expressions"][0]["Column"]["Property"],
        "Area"
    );
    let bar = &report.pages[0].visuals[2];
    assert_eq!(bar.projections[0].query_ref, "Stores.Area");
    assert_eq!(bar.projections[0].native_query_ref.as_deref(), Some("Area"));
    assert_eq!(
        bar.filters[0].field.as_ref().unwrap().property(),
        Some("Area")
    );
    assert_eq!(
        bar.objects["dataPoint"][0]["selector"]["metadata"],
        "Stores.Area"
    );
    assert!(events.contains(&ChangeEvent::ObjectRenamed {
        id: region,
        old: "Region".into(),
        new: "Area".into(),
    }));

    let undo_events = history.undo(&mut doc).unwrap();
    assert!(undo_events.contains(&ChangeEvent::ObjectRenamed {
        id: key,
        old: "Key".into(),
        new: "Store ID".into(),
    }));
    history.undo(&mut doc).unwrap();
    history.undo(&mut doc).unwrap();
    assert_eq!(doc, original);
    assert_eq!(history.undo(&mut doc), Err(CommandError::NothingToUndo));
}

#[test]
fn rename_measure_updates_projections() {
    let mut doc = sample_document();
    let id = measure_id(&doc, "Stores", "Active Stores");
    let command = RenameMeasure {
        measure: id,
        new_name: "Stores With Sales".into(),
    };
    assert_round_trip(&mut doc, command, |doc, events| {
        let card = &doc.reports[0].pages[0].visuals[1];
        assert_eq!(
            card.projections[0].field,
            FieldExpr::measure("Stores", "Stores With Sales")
        );
        assert_eq!(card.projections[0].query_ref, "Stores.Stores With Sales");
        assert_eq!(
            card.projections[0].native_query_ref.as_deref(),
            Some("Stores With Sales")
        );
        assert_eq!(events.len(), 2, "{events:?}");
    });
}

#[test]
fn rename_case_only_and_same_name() {
    let mut doc = sample_document();
    let id = table_id(&doc, "Orders");
    let mut history = History::new();
    history
        .execute(
            &mut doc,
            RenameTable {
                table: id,
                new_name: "ORDERS".into(),
            },
        )
        .unwrap();
    let model = doc.model.as_ref().unwrap();
    assert_eq!(model.relationships[0].from.table, "ORDERS");

    let before = doc.clone();
    let events = history
        .execute(
            &mut doc,
            RenameTable {
                table: id,
                new_name: "ORDERS".into(),
            },
        )
        .unwrap();
    assert!(events.is_empty());
    assert_eq!(doc, before);
}

type ErrorCheck = fn(&CommandError) -> bool;

#[test]
fn invalid_renames_fail_without_changing_the_document() {
    let mut doc = sample_document();
    let original = doc.clone();
    let mut history = History::new();
    let stores = table_id(&doc, "Stores");
    let total = measure_id(&doc, "Stores", "Store Count");
    let key = column_id(&doc, "Stores", "Store ID");

    let conflict: ErrorCheck = |e| matches!(e, CommandError::NameConflict { .. });
    let cases: Vec<(Box<dyn Command>, ErrorCheck)> = vec![
        (
            Box::new(RenameTable {
                table: stores,
                new_name: "orders".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameTable {
                table: stores,
                new_name: "  ".into(),
            }),
            |e| matches!(e, CommandError::InvalidName(_)),
        ),
        (
            Box::new(RenameTable {
                table: key,
                new_name: "X".into(),
            }),
            |e| {
                matches!(
                    e,
                    CommandError::WrongKind {
                        expected: ObjectKind::Table,
                        actual: ObjectKind::Column,
                        ..
                    }
                )
            },
        ),
        (
            Box::new(RenameMeasure {
                measure: total,
                new_name: "ACTIVE STORES".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameMeasure {
                measure: total,
                new_name: "store id".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameColumn {
                column: key,
                new_name: "Store Name".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameColumn {
                column: key,
                new_name: "Active Stores".into(),
            }),
            conflict,
        ),
        (
            Box::new(RenameColumn {
                column: ObjectId(999_999),
                new_name: "X".into(),
            }),
            |e| matches!(e, CommandError::NotFound(_)),
        ),
    ];
    for (command, expected) in cases {
        let label = command.label();
        let err = history.execute_boxed(&mut doc, command).unwrap_err();
        assert!(expected(&err), "{label}: unexpected error {err}");
        assert_eq!(doc, original);
    }
    assert!(!history.can_undo());
    assert!(!history.is_dirty());
}

#[test]
fn property_edits_round_trip() {
    let mut doc = sample_document();
    let active = measure_id(&doc, "Stores", "Active Stores");
    assert_round_trip(
        &mut doc,
        SetMeasureExpression {
            measure: active,
            expression: "COUNTROWS(FILTER(Stores, [Order Count] > 0))".into(),
        },
        |doc, events| {
            let m = doc
                .model
                .as_ref()
                .unwrap()
                .measure("Stores", "Active Stores");
            assert!(m.unwrap().expression.starts_with("COUNTROWS(FILTER"));
            assert_eq!(events, [ChangeEvent::ObjectChanged { id: active }]);
        },
    );

    let region = column_id(&doc, "Stores", "Region");
    assert_round_trip(
        &mut doc,
        SetDescription {
            id: region,
            text: None,
        },
        |doc, _| {
            let c = doc.model.as_ref().unwrap().column("Stores", "Region");
            assert_eq!(c.unwrap().description, None);
        },
    );
    let blob = doc.model.as_ref().unwrap().expressions[0].id;
    assert_round_trip(
        &mut doc,
        SetDescription {
            id: blob,
            text: Some("Container URL".into()),
        },
        |doc, _| {
            assert_eq!(doc.find(blob).unwrap().description(), Some("Container URL"));
        },
    );

    let slicer = doc.reports[0].pages[0].visuals[0].id;
    let moved = Position {
        x: 20.0,
        y: 30.0,
        z: 5.0,
        width: 300.0,
        height: 80.0,
        tab_order: Some(1),
        angle: None,
    };
    assert_round_trip(
        &mut doc,
        SetVisualPosition {
            visual: slicer,
            position: moved.clone(),
        },
        |doc, events| {
            assert_eq!(doc.reports[0].pages[0].visuals[0].position, moved);
            assert_eq!(events, [ChangeEvent::ObjectChanged { id: slicer }]);
        },
    );

    let err = SetDescription {
        id: slicer,
        text: Some("x".into()),
    }
    .apply(&mut doc)
    .unwrap_err();
    assert_eq!(
        err,
        CommandError::Unsupported {
            kind: ObjectKind::Visual,
            operation: "descriptions",
        }
    );
}

#[test]
fn history_tracks_dirty_state_and_labels() {
    let mut doc = sample_document();
    let slicer = doc.reports[0].pages[0].visuals[0].id;
    let move_to = |x: f64| SetVisualPosition {
        visual: slicer,
        position: Position {
            x,
            ..Position::default()
        },
    };
    let mut history = History::new();
    assert!(!history.is_dirty());
    history.execute(&mut doc, move_to(1.0)).unwrap();
    assert!(history.is_dirty());
    assert_eq!(history.undo_label(), Some("Move visual"));
    history.mark_saved();
    assert!(!history.is_dirty());

    history.undo(&mut doc).unwrap();
    assert!(history.is_dirty());
    assert_eq!(history.redo_label(), Some("Move visual"));
    history.redo(&mut doc).unwrap();
    assert!(!history.is_dirty());

    // Branching after an undo drops the redo stack; the saved state is gone.
    history.undo(&mut doc).unwrap();
    history.execute(&mut doc, move_to(2.0)).unwrap();
    assert!(!history.can_redo());
    assert!(history.is_dirty());
    assert_eq!(history.redo(&mut doc), Err(CommandError::NothingToRedo));

    // Clearing keeps the dirty flag.
    history.clear();
    assert!(history.is_dirty());
    assert!(!history.can_undo());
    history.mark_saved();
    assert!(!history.is_dirty());
}

#[test]
fn history_limit_keeps_dirty_tracking_correct() {
    let mut doc = sample_document();
    let slicer = doc.reports[0].pages[0].visuals[0].id;
    let mut history = History::with_limit(2);
    for x in [1.0, 2.0, 3.0] {
        history
            .execute(
                &mut doc,
                SetVisualPosition {
                    visual: slicer,
                    position: Position {
                        x,
                        ..Position::default()
                    },
                },
            )
            .unwrap();
    }
    assert_eq!(history.undo_depth(), 2);
    history.undo(&mut doc).unwrap();
    history.undo(&mut doc).unwrap();
    assert!(!history.can_undo());
    // The first move can no longer be undone, so this is not the saved state.
    assert!(history.is_dirty());
    assert_eq!(doc.reports[0].pages[0].visuals[0].position.x, 1.0);
}

#[test]
fn command_group_is_atomic() {
    let mut doc = sample_document();
    let original = doc.clone();
    let active = measure_id(&doc, "Stores", "Active Stores");
    let stores = table_id(&doc, "Stores");

    let failing = CommandGroup::new("Edit")
        .with(SetMeasureExpression {
            measure: active,
            expression: "1".into(),
        })
        .with(RenameTable {
            table: stores,
            new_name: "Orders".into(),
        });
    let mut history = History::new();
    assert!(matches!(
        history.execute(&mut doc, failing),
        Err(CommandError::NameConflict { .. })
    ));
    assert_eq!(doc, original);

    let group = CommandGroup::new("Edit and rename")
        .with(SetMeasureExpression {
            measure: active,
            expression: "1".into(),
        })
        .with(RenameTable {
            table: stores,
            new_name: "Shops".into(),
        });
    let events = history.execute(&mut doc, group).unwrap();
    assert!(events.iter().any(|e| e.id() == active));
    assert!(events.iter().any(|e| e.id() == stores));
    assert_eq!(history.undo_label(), Some("Edit and rename"));
    let after = doc.clone();
    history.undo(&mut doc).unwrap();
    assert_eq!(doc, original);
    history.redo(&mut doc).unwrap();
    assert_eq!(doc, after);
}

#[test]
fn replace_objects_validates_targets_first() {
    let mut doc = sample_document();
    let original = doc.clone();
    let mut table = doc.model.as_ref().unwrap().tables[0].clone();
    table.is_hidden = true;
    let mut missing = table.clone();
    missing.id = ObjectId(999_999);
    let command = ReplaceObjects {
        label: "Replace".into(),
        items: vec![Snapshot::Table(table), Snapshot::Table(missing)],
    };
    assert_eq!(
        command.apply(&mut doc).unwrap_err(),
        CommandError::NotFound(ObjectId(999_999))
    );
    assert_eq!(doc, original);
}

#[test]
fn change_events_serialize_for_ipc() {
    let event = ChangeEvent::ObjectRenamed {
        id: ObjectId(7),
        old: "a".into(),
        new: "b".into(),
    };
    let json = serde_json::to_value(&event).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"objectRenamed": {"id": 7, "old": "a", "new": "b"}})
    );
    let back: ChangeEvent = serde_json::from_value(json).unwrap();
    assert_eq!(back, event);
}
