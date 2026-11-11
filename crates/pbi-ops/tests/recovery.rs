//! Recovery journals (M8.1): unsaved edits are journaled to the recovery
//! store and `doc.recovery.*` lists, restores and discards them.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::path::Path;

use common::{Runner, mini_pbip, tmp_dir};
use pbi_ops::recovery::RecoveryStore;
use pbi_ops::{OpErrorCode, Registry, Workspace};
use serde_json::{Value, json};

fn runner(store: &Path) -> Runner {
    let ws = Workspace::new();
    ws.set_recovery(Some(RecoveryStore::new(store)));
    Runner::with(Registry::default_catalog(), ws)
}

fn journal_files(store: &Path) -> usize {
    std::fs::read_dir(store).map_or(0, |d| d.count())
}

/// Edits that only replay to the same model when object ids are
/// deterministic: the new measure is addressed by the id it got.
async fn edit(r: &Runner, doc: &str) {
    let total = r.run("model.measures", json!({"docId": doc})).await["measures"][0]["id"].clone();
    r.run(
        "model.object.setDescription",
        json!({"docId": doc, "objectId": total, "text": "Sum of amounts"}),
    )
    .await;
    let created = r
        .run(
            "model.measure.create",
            json!({"docId": doc, "table": "Sales", "name": "Rows", "expression": "COUNTROWS(Sales)"}),
        )
        .await;
    r.run(
        "model.object.setDescription",
        json!({"docId": doc, "objectId": created["measureId"], "text": "Row count"}),
    )
    .await;
    r.run(
        "model.object.rename",
        json!({"docId": doc, "table": "Sales", "measure": "Rows", "newName": "Row Count"}),
    )
    .await;
    r.run("doc.undo", json!({"docId": doc})).await;
    r.run("doc.redo", json!({"docId": doc})).await;
    r.run(
        "model.object.setDescription",
        json!({"docId": doc, "objectId": total, "text": "Sum of all amounts"}),
    )
    .await;
}

fn measures(out: &Value) -> Vec<(String, String)> {
    out["measures"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["name"].as_str().unwrap_or_default().to_owned(),
                m["description"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

#[tokio::test]
async fn unsaved_edits_survive_a_lost_workspace_and_save_ends_the_journal() {
    let dir = tmp_dir("recovery-");
    let pbip = mini_pbip(dir.path());
    let store = dir.path().join("store");

    let before = {
        let r = runner(&store);
        let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
        assert_eq!(journal_files(&store), 0, "no journal before an edit");
        r.run("model.measures", json!({"docId": doc})).await;
        assert_eq!(journal_files(&store), 0, "reads are not journaled");
        edit(&r, &doc).await;
        assert_eq!(journal_files(&store), 1);
        let held = r.run("doc.recovery.list", json!({})).await;
        assert_eq!(held["journals"], json!([]), "an open document holds it");
        measures(&r.run("model.measures", json!({"docId": doc})).await)
        // The workspace is dropped without saving or closing: a crash.
    };
    assert!(
        before.contains(&("Row Count".into(), "Row count".into())),
        "{before:?}"
    );

    let r = runner(&store);
    let list = r.run("doc.recovery.list", json!({})).await;
    assert_eq!(list["enabled"], json!(true));
    let journals = list["journals"].as_array().unwrap();
    assert_eq!(journals.len(), 1, "{list}");
    let journal = &journals[0];
    assert_eq!(
        (
            &journal["edits"],
            &journal["sourceExists"],
            &journal["lastEdit"].is_string()
        ),
        (&json!(7), &json!(true), &true),
        "{journal}"
    );
    assert_eq!(
        Path::new(journal["path"].as_str().unwrap()),
        std::path::absolute(&pbip).unwrap()
    );
    let id = journal["recoveryId"].clone();

    let restored = r
        .run("doc.recovery.restore", json!({"recoveryId": id}))
        .await;
    assert_eq!(
        (
            &restored["edits"],
            &restored["replayed"],
            &restored["dirty"],
            restored.get("failed")
        ),
        (&json!(7), &json!(7), &json!(true), None),
        "{restored}"
    );
    let doc = restored["docId"].as_str().unwrap().to_owned();
    let after = measures(&r.run("model.measures", json!({"docId": doc})).await);
    assert_eq!(after, before);

    // Restored edits keep journaling into the same journal; a reopen is refused.
    let err = r
        .try_run("doc.recovery.restore", json!({"recoveryId": id}))
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::InvalidParams, "{err:?}");
    r.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(journal_files(&store), 1);
    assert_eq!(
        RecoveryStore::new(&store)
            .read(id.as_str().unwrap())
            .unwrap()
            .entries
            .len(),
        8
    );

    r.run("doc.save", json!({"docId": doc})).await;
    assert_eq!(journal_files(&store), 0, "a save ends the journal");
    r.run(
        "model.object.setDescription",
        json!({"docId": doc, "objectId": r.run("model.measures", json!({"docId": doc})).await["measures"][0]["id"], "text": "Again"}),
    )
    .await;
    assert_eq!(
        journal_files(&store),
        1,
        "the next edit starts a new journal"
    );
    r.run("doc.close", json!({"docId": doc, "discardChanges": true}))
        .await;
    assert_eq!(journal_files(&store), 0, "closing discards it");
}

#[tokio::test]
async fn journals_can_be_discarded_and_a_failed_replay_keeps_the_journal() {
    let dir = tmp_dir("recovery-discard-");
    let pbip = mini_pbip(dir.path());
    let store = dir.path().join("store");
    {
        let r = runner(&store);
        let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
        edit(&r, &doc).await;
    }
    // The project changed on disk since: the measure the journal creates
    // already exists, so its replay stops there.
    let tmdl = dir
        .path()
        .join("Mini.SemanticModel/definition/tables/Sales.tmdl");
    let text = std::fs::read_to_string(&tmdl).unwrap();
    std::fs::write(
        &tmdl,
        text.replacen(
            "\tmeasure Total",
            "\tmeasure Rows = 1\n\n\tmeasure Total",
            1,
        ),
    )
    .unwrap();

    let r = runner(&store);
    let id = r.run("doc.recovery.list", json!({})).await["journals"][0]["recoveryId"].clone();
    let restored = r
        .run("doc.recovery.restore", json!({"recoveryId": id}))
        .await;
    assert_eq!(
        (
            &restored["replayed"],
            &restored["failed"]["index"],
            &restored["failed"]["command"]
        ),
        (&json!(1), &json!(1), &json!("model.measure.create")),
        "{restored}"
    );
    let doc = restored["docId"].as_str().unwrap().to_owned();
    r.run("doc.close", json!({"docId": doc, "discardChanges": true}))
        .await;
    assert_eq!(
        journal_files(&store),
        1,
        "a failed replay keeps the journal"
    );

    let out = r
        .run("doc.recovery.discard", json!({"recoveryId": id}))
        .await;
    assert_eq!(out["discarded"], json!(true));
    assert_eq!(journal_files(&store), 0);
    let out = r
        .run("doc.recovery.discard", json!({"recoveryId": id}))
        .await;
    assert_eq!(out["discarded"], json!(false));
    let err = r
        .try_run("doc.recovery.restore", json!({"recoveryId": id}))
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::NotFound, "{err:?}");
}

#[tokio::test]
async fn recovery_is_off_by_default() {
    let dir = tmp_dir("recovery-off-");
    let pbip = mini_pbip(dir.path());
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    edit(&r, &doc).await;
    let list = r.run("doc.recovery.list", json!({})).await;
    assert_eq!(list, json!({"enabled": false, "journals": []}));
    let err = r
        .try_run("doc.recovery.restore", json!({"recoveryId": "abc-1"}))
        .await
        .unwrap_err();
    assert_eq!(err.code, OpErrorCode::Unsupported, "{err:?}");
}

async fn describe(r: &Runner, doc: &str, text: &str) {
    let total = r.run("model.measures", json!({"docId": doc})).await["measures"][0]["id"].clone();
    r.run(
        "model.object.setDescription",
        json!({"docId": doc, "objectId": total, "text": text}),
    )
    .await;
}

#[tokio::test]
async fn only_journals_that_replay_from_the_saved_document_are_kept() {
    let dir = tmp_dir("recovery-untracked-");
    let pbip = mini_pbip(dir.path());
    let store = dir.path().join("store");
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;

    // Edits made before recovery was on are in no journal: later ones alone
    // would not replay, so nothing is journaled until the next save.
    describe(&r, &doc, "Before").await;
    r.ws.set_recovery(Some(RecoveryStore::new(&store)));
    describe(&r, &doc, "After").await;
    assert_eq!(journal_files(&store), 0);
    r.run("doc.save", json!({"docId": doc})).await;
    describe(&r, &doc, "Saved").await;
    assert_eq!(journal_files(&store), 1);

    // Undoing within the journal keeps it; undoing past its start (an edit
    // saved before it) drops it.
    r.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(journal_files(&store), 1);
    r.run("doc.undo", json!({"docId": doc})).await;
    assert_eq!(
        journal_files(&store),
        0,
        "the undo reached before the journal"
    );
    describe(&r, &doc, "Untracked").await;
    assert_eq!(journal_files(&store), 0);

    // Turning recovery off deletes the journals of open documents.
    r.run("doc.save", json!({"docId": doc})).await;
    describe(&r, &doc, "Journaled").await;
    assert_eq!(journal_files(&store), 1);
    r.ws.set_recovery(None);
    assert_eq!(journal_files(&store), 0);
}
