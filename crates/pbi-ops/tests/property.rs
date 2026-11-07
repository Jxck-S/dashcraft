//! Property tests through `Registry::execute` (M6.11):
//!
//! - every catalog command called with its examples, with parameters derived
//!   from its JSON Schema and with mutated parameters: no panic (which the
//!   registry reports as an `internal` error) and no hang;
//! - seeded random edit, undo and redo sequences on the synthetic Acme
//!   Outfitters example: a failed or no-op edit changes nothing, the edited
//!   model saves and reopens with the same shape, and undoing everything
//!   restores a document equal to the original (M6 acceptance 10) with the
//!   same measure values.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::{BLOB_MAP, PBIP, Runner, fixture, mini_pbip, tmp_dir};
use pbi_model::Document;
use pbi_ops::{CommandInfo, OpError, OpErrorCode, PathPolicy, Registry, Workspace};
use serde_json::{Map, Value, json};

/// xorshift64*: deterministic and dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    /// `true` with probability `percent` / 100.
    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }

    fn pick<T: Clone>(&mut self, items: &[T]) -> Option<T> {
        if items.is_empty() {
            None
        } else {
            Some(items[self.below(items.len())].clone())
        }
    }
}

const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// Runs one command, failing the test on a hang or an `internal` error.
async fn call(r: &Runner, id: &str, params: &Value) -> Result<Value, OpError> {
    let result = tokio::time::timeout(CALL_TIMEOUT, r.try_run(id, params.clone()))
        .await
        .unwrap_or_else(|_| panic!("{id} did not finish in {CALL_TIMEOUT:?}: {params:.300}"));
    if let Err(e) = &result {
        assert_ne!(e.code, OpErrorCode::Internal, "{id} {params:.300}: {e:?}");
    }
    result
}

// ---------------------------------------------------------------------------
// Every command with derived and mutated parameters
// ---------------------------------------------------------------------------

/// Strings that name objects of the mini project and common inputs.
const NAMES: &[&str] = &[
    "Sales",
    "Total",
    "Region",
    "Amount",
    "Item",
    "d1",
    "Mini.pbip",
    "1",
    "East",
    "png",
];

/// Values that probe types, bounds, paths and text handling.
fn odd(rng: &mut Rng) -> Value {
    match rng.below(20) {
        0 => Value::Null,
        1 => json!(true),
        2 => json!(-1),
        3 => json!(0),
        4 => json!(u64::MAX),
        5 => json!(i64::MIN),
        6 => json!(1e308),
        7 => json!(-0.5),
        8 => json!(""),
        9 => json!("../../outside.pbip"),
        10 => json!("/"),
        11 => json!("'"),
        12 => json!("]["),
        13 => json!("\u{0}\u{7f}"),
        14 => json!("é😀".repeat(40)),
        15 => json!("x".repeat(10_000)),
        16 => json!([]),
        17 => json!([1, "a", null, {}]),
        18 => json!({}),
        _ => json!({"a": [{"b": null}]}),
    }
}

/// A value for `schema` (as schemars writes it), resolving `$ref`s against
/// `root`; sometimes an odd value instead.
fn derive(schema: &Value, root: &Value, rng: &mut Rng, depth: usize) -> Value {
    if depth > 6 || rng.chance(8) {
        return odd(rng);
    }
    if let Some(r) = schema.get("$ref").and_then(Value::as_str) {
        let name = r.rsplit('/').next().unwrap_or_default();
        let target = root
            .get("$defs")
            .or_else(|| root.get("definitions"))
            .and_then(|d| d.get(name));
        return match target {
            Some(t) => derive(t, root, rng, depth + 1),
            None => odd(rng),
        };
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        return rng.pick(values).unwrap_or(Value::Null);
    }
    if let Some(c) = schema.get("const") {
        return c.clone();
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(branch) = schema
            .get(key)
            .and_then(Value::as_array)
            .and_then(|b| rng.pick(b))
        {
            return derive(&branch, root, rng, depth + 1);
        }
    }
    let ty = match schema.get("type") {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(types)) => rng
            .pick(types)
            .and_then(|t| t.as_str().map(str::to_owned))
            .unwrap_or_default(),
        _ if schema.get("properties").is_some() => "object".to_owned(),
        _ => String::new(),
    };
    match ty.as_str() {
        "null" => Value::Null,
        "boolean" => json!(rng.chance(50)),
        // Small: ids, indexes and limits of the mini project.
        "integer" => json!(rng.below(12)),
        "number" => json!(rng.below(2000) as f64 - 500.0),
        "string" => json!(rng.pick(NAMES).unwrap_or_default()),
        "array" => {
            let item = schema.get("items").cloned().unwrap_or_else(|| json!({}));
            Value::Array(
                (0..rng.below(4))
                    .map(|_| derive(&item, root, rng, depth + 1))
                    .collect(),
            )
        }
        "object" => {
            let required: Vec<&str> = schema
                .get("required")
                .and_then(Value::as_array)
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let mut out = Map::new();
            if let Some(props) = schema.get("properties").and_then(Value::as_object) {
                for (key, s) in props {
                    if required.contains(&key.as_str()) || rng.chance(50) {
                        out.insert(key.clone(), derive(s, root, rng, depth + 1));
                    }
                }
            }
            Value::Object(out)
        }
        _ => odd(rng),
    }
}

/// One random change at a random depth: a key removed, a value replaced by
/// an odd one, an unknown key added or an array item removed.
fn mutate(value: &mut Value, rng: &mut Rng) {
    match value {
        Value::Object(map) if !map.is_empty() => {
            let key = map.keys().nth(rng.below(map.len())).cloned().unwrap();
            match rng.below(4) {
                0 => {
                    map.remove(&key);
                }
                1 => {
                    map.insert(key, odd(rng));
                }
                2 => {
                    map.insert("unexpected".to_owned(), odd(rng));
                }
                _ => mutate(map.get_mut(&key).unwrap(), rng),
            }
        }
        Value::Array(items) if !items.is_empty() => {
            let i = rng.below(items.len());
            if rng.chance(30) {
                items.remove(i);
            } else {
                mutate(&mut items[i], rng);
            }
        }
        other => *other = odd(rng),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn every_command_survives_derived_and_mutated_params() {
    const MUTATIONS: usize = 40;
    const DERIVED: usize = 60;
    let dir = tmp_dir("pbi-ops-prop-commands-");
    let root = dir.path().to_path_buf();
    mini_pbip(&root);
    let policy = PathPolicy::restricted([&root], [&root]).unwrap();
    let r = Runner::with(Registry::default_catalog(), Workspace::with_policy(policy));
    let mut live = r.open(json!({"path": "Mini.pbip"})).await;
    let mut rng = Rng(0x4d36_2e31_3131);
    let infos: Vec<CommandInfo> = r
        .registry
        .list()
        .iter()
        .map(|spec| CommandInfo::of(spec, true))
        .collect();
    let (mut calls, mut succeeded) = (0, 0);
    for info in &infos {
        let schema = info.params_schema.clone().unwrap_or_else(|| json!({}));
        let mut cases = Vec::new();
        for example in &info.examples {
            cases.push(example.params.clone());
            for _ in 0..MUTATIONS {
                let mut params = example.params.clone();
                mutate(&mut params, &mut rng);
                cases.push(params);
            }
        }
        for _ in 0..DERIVED {
            cases.push(derive(&schema, &schema, &mut rng, 0));
        }
        for mut params in cases {
            if let Some(doc) = params.get_mut("docId")
                && (*doc == "d1" || rng.chance(70))
            {
                *doc = json!(live);
            }
            succeeded += usize::from(call(&r, &info.id, &params).await.is_ok());
            calls += 1;
            // Keep one open document: drop the others, reopen after a close.
            for entry in r.ws.documents() {
                if entry.id() != live {
                    let _ = r.ws.remove(entry.id());
                }
            }
            if r.ws.find(&live).is_none() {
                live = r.open(json!({"path": "Mini.pbip"})).await;
            }
        }
    }
    println!(
        "{calls} calls ({succeeded} succeeded) over {} commands",
        infos.len()
    );
    assert!(calls > infos.len() * DERIVED && succeeded > calls / 10);
}

// ---------------------------------------------------------------------------
// Random edit, undo and redo sequences
// ---------------------------------------------------------------------------

/// Objects to aim random edits at, read from the current document.
#[derive(Default)]
struct Targets {
    tables: Vec<(u64, String)>,
    calculated_tables: Vec<u64>,
    /// `(id, table, column)`.
    columns: Vec<(u64, String, String)>,
    /// `(id, table, measure)`.
    measures: Vec<(u64, String, String)>,
    relationships: Vec<u64>,
    hierarchies: Vec<u64>,
    /// Shared expression names.
    expressions: Vec<String>,
    /// `(report, page, visual)`.
    visuals: Vec<(String, String, String)>,
}

impl Targets {
    fn of(doc: &Document) -> Self {
        let mut t = Self::default();
        if let Some(model) = &doc.model {
            for table in &model.tables {
                t.tables.push((table.id.0, table.name.clone()));
                if table.calculated_expression().is_some() {
                    t.calculated_tables.push(table.id.0);
                }
                for c in &table.columns {
                    t.columns.push((c.id.0, table.name.clone(), c.name.clone()));
                }
                for m in &table.measures {
                    t.measures
                        .push((m.id.0, table.name.clone(), m.name.clone()));
                }
                t.hierarchies
                    .extend(table.hierarchies.iter().map(|h| h.id.0));
            }
            t.relationships
                .extend(model.relationships.iter().map(|r| r.id.0));
            t.expressions
                .extend(model.expressions.iter().map(|e| e.name.clone()));
        }
        for report in &doc.reports {
            for page in &report.pages {
                for v in &page.visuals {
                    t.visuals
                        .push((report.name.clone(), page.name.clone(), v.name.clone()));
                }
            }
        }
        t
    }

    fn table(&self, rng: &mut Rng) -> String {
        rng.pick(&self.tables)
            .map_or_else(|| "Nope".to_owned(), |t| t.1)
    }

    /// A column, often of `table`.
    fn column(&self, rng: &mut Rng, table: Option<&str>) -> (u64, String, String) {
        let of_table: Vec<_> = match table {
            Some(name) if rng.chance(80) => self
                .columns
                .iter()
                .filter(|c| c.1 == name)
                .cloned()
                .collect(),
            _ => self.columns.clone(),
        };
        rng.pick(&of_table)
            .unwrap_or_else(|| (999_999, "Nope".to_owned(), "Nope".to_owned()))
    }

    fn expression(&self, rng: &mut Rng) -> String {
        rng.pick(&self.expressions)
            .unwrap_or_else(|| "Nope".to_owned())
    }

    fn measure(&self, rng: &mut Rng) -> (u64, String, String) {
        rng.pick(&self.measures)
            .unwrap_or_else(|| (999_999, "Nope".to_owned(), "Nope".to_owned()))
    }

    fn object_id(&self, rng: &mut Rng) -> u64 {
        match rng.below(4) {
            0 => rng.pick(&self.tables).map_or(999_999, |t| t.0),
            1 => self.column(rng, None).0,
            2 => self.measure(rng).0,
            _ => rng.below(400) as u64,
        }
    }
}

/// Names that are free, taken, or odd for DAX and TMDL.
const ODD_NAMES: &[&str] = &[
    "Total Sales",
    "Orders",
    "Region",
    " padded ",
    "it's",
    "a]b",
    "Ünïcödé ✓",
    "",
    "x",
    "Line Amount",
    "[Bracketed]",
    "tab\there",
    "Source",
    "Text.Upper",
    "say \"hi\"",
    "fnCleanKey",
];

fn new_name(rng: &mut Rng) -> String {
    if rng.chance(65) {
        format!("Fuzz {}", rng.below(40))
    } else {
        rng.pick(ODD_NAMES).unwrap_or_default().to_owned()
    }
}

fn table_ref(table: &str) -> String {
    format!("'{}'", table.replace('\'', "''"))
}

fn column_ref(table: &str, column: &str) -> String {
    format!("{}[{}]", table_ref(table), column.replace(']', "]]"))
}

fn measure_expression(t: &Targets, rng: &mut Rng) -> String {
    let (_, table, column) = t.column(rng, None);
    match rng.below(8) {
        0 => "1".to_owned(),
        1 => "BLANK()".to_owned(),
        2 => format!("COUNTROWS({})", table_ref(&table)),
        3 => format!("SUM({})", column_ref(&table, &column)),
        4 => format!("[{}] * 2", t.measure(rng).2.replace(']', "]]")),
        5 => format!("DISTINCTCOUNT({})", column_ref(&table, &column)),
        6 => "SUM(".to_owned(),
        _ => "[No Such Measure]".to_owned(),
    }
}

fn column_expression(t: &Targets, rng: &mut Rng, table: &str) -> String {
    let (_, of, column) = t.column(rng, Some(table));
    match rng.below(6) {
        0 => "1".to_owned(),
        1 => column_ref(&of, &column),
        2 => format!("LEN({})", column_ref(&of, &column)),
        3 => "\"text\"".to_owned(),
        4 => format!("RELATED({})", column_ref(&of, &column)),
        _ => "(".to_owned(),
    }
}

fn table_expression(t: &Targets, rng: &mut Rng) -> String {
    let (_, table, column) = t.column(rng, None);
    match rng.below(7) {
        0 => format!("VALUES({})", column_ref(&table, &column)),
        1 => format!("DISTINCT({})", column_ref(&table, &column)),
        2 => "ROW(\"A\", 1)".to_owned(),
        3 => "{1, 2, 3}".to_owned(),
        4 => format!("FILTER({}, TRUE())", table_ref(&table)),
        5 => "1".to_owned(),
        _ => "VALUES(".to_owned(),
    }
}

/// A random single edit (often an invalid one): the command id and its
/// parameters without `docId`.
fn random_edit(t: &Targets, rng: &mut Rng) -> (&'static str, Value) {
    let filters = ["oneDirection", "bothDirections", "automatic", "sideways"];
    match rng.below(19) {
        0 => {
            let mut p = json!({"table": t.table(rng), "name": new_name(rng),
                "expression": measure_expression(t, rng)});
            if rng.chance(30) {
                p["formatString"] = json!(rng.pick(&["#,0", "0.0%", ""]).unwrap_or_default());
            }
            ("model.measure.create", p)
        }
        1 => {
            let mut p = json!({"measureId": t.measure(rng).0});
            match rng.below(5) {
                0 => p["expression"] = json!(measure_expression(t, rng)),
                1 => p["formatString"] = json!("$#,0.00"),
                2 => p["displayFolder"] = json!(new_name(rng)),
                3 => p["isHidden"] = json!(rng.chance(50)),
                _ => p["description"] = json!(new_name(rng)),
            }
            ("model.measure.update", p)
        }
        2 => (
            "model.measure.delete",
            json!({"measureId": t.measure(rng).0}),
        ),
        3 => (
            "model.setMeasureExpression",
            json!({"measureId": t.measure(rng).0, "expression": measure_expression(t, rng)}),
        ),
        4 => {
            let p = match rng.below(4) {
                0 => json!({"table": t.table(rng), "newName": new_name(rng)}),
                1 => {
                    let (_, table, column) = t.column(rng, None);
                    json!({"table": table, "column": column, "newName": new_name(rng)})
                }
                2 => json!({"expression": t.expression(rng), "newName": new_name(rng)}),
                _ => {
                    let (_, table, measure) = t.measure(rng);
                    json!({"table": table, "measure": measure, "newName": new_name(rng)})
                }
            };
            ("model.object.rename", p)
        }
        5 => {
            let text = match rng.below(4) {
                0 => Value::Null,
                1 => json!("two\nlines"),
                2 => json!("A note ✓"),
                _ => json!(new_name(rng)),
            };
            (
                "model.object.setDescription",
                json!({"objectId": t.object_id(rng), "text": text}),
            )
        }
        6 => {
            let (_, ft, fc) = t.column(rng, None);
            let (_, tt, tc) = t.column(rng, None);
            let mut p =
                json!({"from": {"table": ft, "column": fc}, "to": {"table": tt, "column": tc}});
            if rng.chance(50) {
                p["crossFilteringBehavior"] = json!(rng.pick(&filters).unwrap_or_default());
            }
            if rng.chance(50) {
                p["isActive"] = json!(rng.chance(50));
            }
            ("model.relationship.create", p)
        }
        7 => {
            let id = rng.pick(&t.relationships).unwrap_or(999_999);
            let p = if rng.chance(50) {
                json!({"relationshipId": id, "isActive": rng.chance(50)})
            } else {
                json!({"relationshipId": id,
                    "crossFilteringBehavior": rng.pick(&filters).unwrap_or_default()})
            };
            ("model.relationship.update", p)
        }
        8 => (
            "model.relationship.delete",
            json!({"relationshipId": rng.pick(&t.relationships).unwrap_or(999_999)}),
        ),
        9 => {
            let table = t.table(rng);
            let expression = column_expression(t, rng, &table);
            (
                "model.column.create",
                json!({"table": table, "name": new_name(rng), "expression": expression}),
            )
        }
        10 => (
            "model.column.delete",
            json!({"columnId": t.column(rng, None).0}),
        ),
        11 => (
            "model.table.create",
            json!({"name": new_name(rng), "expression": table_expression(t, rng)}),
        ),
        12 => (
            "model.table.delete",
            json!({"tableId": rng.pick(&t.tables).map_or(999_999, |t| t.0)}),
        ),
        13 => {
            let table = t.table(rng);
            let levels: Vec<Value> = (0..=rng.below(3))
                .map(|_| json!({"column": t.column(rng, Some(&table)).2}))
                .collect();
            (
                "model.hierarchy.create",
                json!({"table": table, "name": new_name(rng), "levels": levels}),
            )
        }
        14 => (
            "model.hierarchy.delete",
            json!({"hierarchyId": rng.pick(&t.hierarchies).unwrap_or(999_999)}),
        ),
        15 => {
            let (report, page, visual) = rng
                .pick(&t.visuals)
                .unwrap_or_else(|| ("1".to_owned(), "1".to_owned(), "nope".to_owned()));
            let coord = |rng: &mut Rng| json!(rng.below(1400) as f64 - 100.0 + 0.5);
            if rng.chance(50) {
                let p = json!({"report": report, "page": page, "visual": visual,
                    "x": coord(rng), "y": coord(rng)});
                ("report.visual.move", p)
            } else {
                let p = json!({"report": report, "page": page, "visual": visual,
                    "width": coord(rng), "height": coord(rng)});
                ("report.visual.resize", p)
            }
        }
        16 => {
            let (id, table, _) = t.column(rng, None);
            let mut p = json!({"columnId": id});
            match rng.below(6) {
                0 | 1 => p["expression"] = json!(column_expression(t, rng, &table)),
                2 => {
                    let types = [
                        "string", "int64", "double", "boolean", "dateTime", "decimal",
                    ];
                    p["dataType"] = json!(rng.pick(&types).unwrap_or_default());
                }
                3 => p["formatString"] = json!("0.0"),
                4 => p["isHidden"] = json!(rng.chance(50)),
                _ => p["description"] = json!(new_name(rng)),
            }
            ("model.column.update", p)
        }
        17 => {
            let mut p = json!({"tableId": rng.pick(&t.tables).map_or(999_999, |t| t.0)});
            match rng.below(4) {
                0 | 1 => {
                    if let Some(id) = rng.pick(&t.calculated_tables) {
                        p["tableId"] = json!(id);
                    }
                    p["expression"] = json!(table_expression(t, rng));
                }
                2 => p["isHidden"] = json!(rng.chance(50)),
                _ => p["description"] = json!(new_name(rng)),
            }
            ("model.table.update", p)
        }
        _ => {
            let edits: Vec<Value> = (0..2 + rng.below(2))
                .map(|_| {
                    let (id, mut p) = random_edit(t, rng);
                    p["type"] = json!(edit_type(id));
                    p
                })
                .collect();
            ("doc.applyEdits", json!({"edits": edits}))
        }
    }
}

/// The `doc.applyEdits` operation of a command.
fn edit_type(command: &str) -> &'static str {
    match command {
        "model.measure.create" => "createMeasure",
        "model.measure.update" => "updateMeasure",
        "model.measure.delete" => "deleteMeasure",
        "model.setMeasureExpression" => "setMeasureExpression",
        "model.object.rename" => "renameObject",
        "model.object.setDescription" => "setDescription",
        "model.relationship.create" => "createRelationship",
        "model.relationship.update" => "updateRelationship",
        "model.relationship.delete" => "deleteRelationship",
        "model.column.create" => "createColumn",
        "model.column.update" => "updateColumn",
        "model.column.delete" => "deleteColumn",
        "model.table.create" => "createTable",
        "model.table.update" => "updateTable",
        "model.table.delete" => "deleteTable",
        "model.hierarchy.create" => "createHierarchy",
        "model.hierarchy.delete" => "deleteHierarchy",
        "report.visual.move" | "report.visual.resize" => "setVisualPosition",
        // A nested group is not an operation: refused as invalid.
        _ => "applyEdits",
    }
}

#[derive(Debug, Default)]
struct Tally {
    committed: usize,
    unchanged: usize,
    refused: usize,
}

/// `steps` random edits (some invalid), undos and redos on `doc`. A refused
/// or no-op step must leave the document as it was.
async fn random_steps(r: &Runner, doc: &str, steps: usize, seed: u64) -> Tally {
    let entry = r.ws.get(doc).unwrap();
    let seed = mixed_seed(seed);
    eprintln!("{steps} random steps, seed {seed:#x}");
    let mut rng = Rng(seed);
    let mut tally = Tally::default();
    let started = std::time::Instant::now();
    let mut timings: std::collections::BTreeMap<&str, (usize, Duration)> = Default::default();
    for step in 0..steps {
        let before = entry.session().document();
        let roll = rng.below(100);
        let (id, mut params) = if roll < 18 {
            ("doc.undo", json!({}))
        } else if roll < 26 {
            ("doc.redo", json!({}))
        } else {
            random_edit(&Targets::of(&before), &mut rng)
        };
        params["docId"] = json!(doc);
        let call_started = std::time::Instant::now();
        let result = call(r, id, &params).await;
        let spent = timings.entry(id).or_insert((0, Duration::ZERO));
        spent.0 += 1;
        spent.1 += call_started.elapsed();
        let after = entry.session().document();
        let changed = match &result {
            Ok(out) => out["changed"] != false,
            Err(_) => false,
        };
        if step % 1000 == 999 {
            eprintln!(
                "step {}: {:?} after {:?}",
                step + 1,
                tally,
                started.elapsed()
            );
        }
        if changed {
            tally.committed += 1;
        } else {
            if !Arc::ptr_eq(&before, &after) && *before != *after {
                let path = first_difference(
                    &serde_json::to_value(&*before).unwrap(),
                    &serde_json::to_value(&*after).unwrap(),
                    "",
                );
                panic!(
                    "step {step}: {id} {params} did not commit ({result:?}) but changed {path:?}"
                );
            }
            if result.is_ok() {
                tally.unchanged += 1;
            } else {
                tally.refused += 1;
            }
        }
    }
    let mut slowest: Vec<_> = timings.into_iter().collect();
    slowest.sort_by_key(|(_, (_, spent))| std::cmp::Reverse(*spent));
    for (id, (n, spent)) in slowest.iter().take(5) {
        eprintln!(
            "  {id}: {n} calls, {spent:?} ({:?} each)",
            *spent / u32::try_from(*n).unwrap_or(1)
        );
    }
    tally
}

/// Undoes every edit of `doc`; returns the number of undos.
async fn undo_all(r: &Runner, doc: &str) -> usize {
    let mut undos = 0;
    while r.run("doc.history", json!({"docId": doc})).await["canUndo"] == true {
        r.run("doc.undo", json!({"docId": doc})).await;
        undos += 1;
    }
    undos
}

/// The first JSON path where `a` and `b` differ.
fn first_difference(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                let p = format!("{path}/{k}");
                match y.get(k) {
                    Some(w) => {
                        if let Some(d) = first_difference(v, w, &p) {
                            return Some(d);
                        }
                    }
                    None => return Some(format!("{p} (removed)")),
                }
            }
            y.keys()
                .find(|k| !x.contains_key(*k))
                .map(|k| format!("{path}/{k} (added)"))
        }
        (Value::Array(x), Value::Array(y)) => {
            for (i, (v, w)) in x.iter().zip(y).enumerate() {
                if let Some(d) = first_difference(v, w, &format!("{path}/{i}")) {
                    return Some(d);
                }
            }
            (x.len() != y.len()).then(|| format!("{path} ({} → {} items)", x.len(), y.len()))
        }
        _ => (a != b).then(|| format!("{path}: {a:.200} → {b:.200}")),
    }
}

/// `doc` against `original`, ignoring the id generator (ids are never
/// reused, so it moves on) and `source` (a save as moves the document).
fn assert_equal_documents(original: &Document, doc: &Document) {
    let mut doc = doc.clone();
    doc.ids = original.ids.clone();
    doc.source = original.source.clone();
    if doc != *original {
        let path = first_difference(
            &serde_json::to_value(original).unwrap(),
            &serde_json::to_value(&doc).unwrap(),
            "",
        );
        panic!("the document differs from the original at {path:?}");
    }
}

/// `seed` mixed with `DASHCRAFT_PROPERTY_SEED` when set: the nightly job
/// varies it to walk new paths; rerun with the same value to reproduce.
fn mixed_seed(seed: u64) -> u64 {
    let extra = std::env::var("DASHCRAFT_PROPERTY_SEED")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    match seed ^ extra.wrapping_mul(0x9e37_79b9_7f4a_7c15) {
        0 => seed,
        mixed => mixed,
    }
}

/// Steps of a random edit test: `DASHCRAFT_PROPERTY_STEPS` if set, else
/// `release` in optimized builds (the nightly job runs `--release`) and a
/// quarter of it in debug builds (CI), where each edit is about ten times
/// slower.
fn steps(release: usize) -> usize {
    std::env::var("DASHCRAFT_PROPERTY_STEPS")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(if cfg!(debug_assertions) {
            release / 4
        } else {
            release
        })
}

/// M6 acceptance 10: 10,000 random edit, undo and redo steps on Acme end,
/// after undoing everything, in a document equal to the original.
#[tokio::test(flavor = "multi_thread")]
async fn random_edits_undo_to_the_original_document() {
    let Some(pbip) = fixture(PBIP) else {
        return;
    };
    let r = Runner::new();
    let doc = r.open(json!({"path": pbip, "autoRefresh": false})).await;
    let entry = r.ws.get(&doc).unwrap();
    let original: Arc<Document> = entry.session().document();
    let steps = steps(10_000);
    let tally = random_steps(&r, &doc, steps, 0x0a11_ce5e_ed00_0010).await;
    println!("{steps} steps: {tally:?}");
    assert!(
        tally.committed > steps / 10 && tally.refused > steps / 100 && tally.unchanged > 0,
        "{tally:?}"
    );
    let undos = undo_all(&r, &doc).await;
    assert!(undos > 0);
    assert_equal_documents(&original, &entry.session().document());
    let history = r.run("doc.history", json!({"docId": doc})).await;
    assert_eq!(history["dirty"], false, "{history}");
}

/// Names, types, expressions and positions: what a save writes and a reopen
/// reads back (ids, origins and lineage are not compared).
fn shape(doc: &Document) -> Value {
    let Some(model) = &doc.model else {
        return Value::Null;
    };
    let tables: Vec<Value> = model
        .tables
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "hidden": t.is_hidden,
                "description": t.description,
                "columns": t.columns.iter().map(|c| json!({
                    "name": c.name, "dataType": c.data_type, "kind": c.kind,
                    "hidden": c.is_hidden, "description": c.description,
                    "formatString": c.format_string,
                })).collect::<Vec<_>>(),
                "measures": t.measures.iter().map(|m| json!({
                    "name": m.name, "expression": m.expression,
                    "formatString": m.format_string, "displayFolder": m.display_folder,
                    "hidden": m.is_hidden, "description": m.description,
                })).collect::<Vec<_>>(),
                "hierarchies": t.hierarchies.iter().map(|h| json!({
                    "name": h.name,
                    "levels": h.levels.iter().map(|l| json!([l.name, l.column])).collect::<Vec<_>>(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let relationships: Vec<Value> = model
        .relationships
        .iter()
        .map(|r| {
            json!({"name": r.name, "from": r.from, "to": r.to, "active": r.is_active,
                "crossFilter": r.cross_filtering_behavior})
        })
        .collect();
    let visuals: Vec<Value> = doc
        .reports
        .iter()
        .flat_map(|report| {
            report.pages.iter().flat_map(move |page| {
                page.visuals
                    .iter()
                    .map(move |v| json!([report.name, page.name, v.name, v.position]))
            })
        })
        .collect();
    json!({"tables": tables, "relationships": relationships, "visuals": visuals})
}

/// Measure values with the captured revisions left out.
async fn measure_values(r: &Runner, doc: &str) -> Value {
    fn strip(v: &mut Value) {
        match v {
            Value::Object(map) => {
                map.remove("documentRevision");
                map.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    let mut out = r.run("measures.values", json!({"docId": doc})).await;
    strip(&mut out);
    out
}

/// Random steps on the loaded example: the edited model saves and reopens
/// with the same shape, and after undoing everything the document and every
/// measure value equal the original.
#[tokio::test(flavor = "multi_thread")]
async fn random_edits_on_loaded_data_save_reopen_and_undo() {
    let (Some(pbip), Some(map)) = (fixture(PBIP), fixture(BLOB_MAP)) else {
        return;
    };
    let r = Runner::new();
    let doc = r
        .open(json!({"path": pbip, "blobMap": map, "source": "query"}))
        .await;
    let entry = r.ws.get(&doc).unwrap();
    let original: Arc<Document> = entry.session().document();
    let values = measure_values(&r, &doc).await;
    let steps = steps(1_000);
    let tally = random_steps(&r, &doc, steps, 0x0a11_ce5e_ed00_0600).await;
    println!("{steps} steps: {tally:?}");
    assert!(tally.committed > steps / 10, "{tally:?}");

    let dir = tmp_dir("pbi-ops-prop-saved-");
    let saved = r
        .run(
            "doc.saveAs",
            json!({"docId": doc, "outDir": dir.path().join("acme")}),
        )
        .await;
    let reopened = Runner::new();
    let pbip_file = dir
        .path()
        .join("acme")
        .join(saved["pbipFile"].as_str().unwrap());
    let copy = reopened
        .open(json!({"path": pbip_file, "autoRefresh": false}))
        .await;
    let edited = shape(&entry.session().document());
    let read_back = shape(&reopened.ws.get(&copy).unwrap().session().document());
    if let Some(path) = first_difference(&edited, &read_back, "") {
        panic!("the saved model reads back differently at {path}");
    }

    undo_all(&r, &doc).await;
    // The save as read the new project back: what describes the source
    // files (diagnostics, unread parts, dataset references) is now the
    // saved project's.
    let mut undone = (*entry.session().document()).clone();
    undone.diagnostics.clone_from(&original.diagnostics);
    undone.extra_parts.clone_from(&original.extra_parts);
    undone.model_status.clone_from(&original.model_status);
    for (report, before) in undone.reports.iter_mut().zip(&original.reports) {
        report
            .dataset_reference
            .clone_from(&before.dataset_reference);
    }
    assert_equal_documents(&original, &undone);
    assert_eq!(measure_values(&r, &doc).await, values);
}
