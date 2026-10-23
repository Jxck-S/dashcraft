//! VertiPaq Analyzer exports (`.vpax`): an OPC ZIP with
//!
//! - `DaxModel.json`: `Dax.Metadata.Model` serialized by Newtonsoft.Json with
//!   `PreserveReferencesHandling.All` — every object has a `"$id"`, repeated
//!   objects are `{"$ref": "<id>"}`, arrays are `{"$id", "$values": […]}`,
//!   default values are omitted, names (`DaxName`) are plain strings
//!   (older files: `{"Name": …}`), notes (`DaxNote`) and expressions
//!   (`DaxExpression`) are strings too. Tables (`TableName`, `RowsCount`,
//!   `IsHidden`, `Columns`, `Measures`, `UserHierarchies`, `Partitions`),
//!   columns (`ColumnName`, `ColumnCardinality`, `DataType`, `ColumnType`,
//!   `Encoding`, `DictionarySize`, `DataSize`, `HierarchiesSize`,
//!   `TotalSize`, `ColumnSegments[].UsedSize`, `ColumnHierarchies[].UsedSize`),
//!   relationships (`FromColumn`/`ToColumn` as column references,
//!   `UsedSizeFrom`, `UsedSizeTo`, `MissingKeys`, `InvalidRows`);
//! - `DaxVpaView.json`: a flat view (`Tables[]`, `Columns[]`,
//!   `ColumnsSegments[]`, `Relationships[]`, … with `TableName`/`ColumnName`
//!   and precomputed sizes), used when `DaxModel.json` is missing or broken;
//! - `Model.bim` (optional): the TOM database (TMSL), the preferred model
//!   source;
//! - `CompatabilityMode.txt` (sic, optional).
//!
//! The document model comes from `Model.bim` when present, else from the
//! names, types and DAX in `DaxModel.json` (no M: the export has no partition
//! queries). Statistics go to [`FormatOutline::Vpax`]. Schema reference:
//! `Dax.Metadata` / `Dax.Vpax` / `Dax.ViewVpaExport` of VertiPaq Analyzer
//! (github.com/sql-bi/VertiPaq-Analyzer, MIT).

use std::collections::HashMap;
use std::sync::Arc;

use pbi_model::{
    Cardinality, Column, ColumnKind, ColumnRef, CrossFilter, DataType, Database, Diagnostic,
    Hierarchy, Level, Measure, Model, Origin, Partition, PartitionMode, PartitionSource,
    Relationship, SourceFormat, Table,
};
use serde_json::Value;

use super::Builder;
use super::codes;
use super::files::{read_json_capped, read_text_capped, root_file};
use super::model::load_tmsl;
use super::outline::{
    FormatOutline, VpaxColumnStats, VpaxRelationshipStats, VpaxStats, VpaxTableStats,
};

/// Largest `DaxModel.json` / `DaxVpaView.json` parsed.
const VPAX_JSON_MAX: u64 = 256 * 1024 * 1024;
/// Most JSON nodes indexed for `$id` resolution.
const MAX_NODES: usize = 20_000_000;

static NULL: Value = Value::Null;

/// `$id` index of a Newtonsoft "preserve references" document.
struct Refs<'a> {
    ids: HashMap<&'a str, &'a Value>,
}

impl<'a> Refs<'a> {
    fn new(root: &'a Value) -> Self {
        let mut ids = HashMap::new();
        let mut stack = vec![root];
        let mut seen = 0usize;
        while let Some(v) = stack.pop() {
            seen += 1;
            if seen > MAX_NODES {
                break;
            }
            match v {
                Value::Object(map) => {
                    if let Some(Value::String(id)) = map.get("$id") {
                        ids.entry(id.as_str()).or_insert(v);
                    }
                    stack.extend(map.values());
                }
                Value::Array(items) => stack.extend(items.iter()),
                _ => {}
            }
        }
        Self { ids }
    }

    /// Follows a `{"$ref": id}` (one level; references never chain).
    fn get(&self, v: &'a Value) -> &'a Value {
        match v.get("$ref").and_then(Value::as_str) {
            Some(id) => self.ids.get(id).copied().unwrap_or(&NULL),
            None => v,
        }
    }

    /// Items of an array property (plain array or `{"$values": [...]}`).
    fn list(&self, v: Option<&'a Value>) -> impl Iterator<Item = &'a Value> + '_ {
        let items: &'a [Value] = match v.map(|v| self.get(v)) {
            Some(Value::Array(items)) => items,
            Some(Value::Object(map)) => map
                .get("$values")
                .and_then(Value::as_array)
                .map_or(&[], Vec::as_slice),
            _ => &[],
        };
        items.iter().map(|item| self.get(item))
    }
}

/// A `DaxName`/`DaxNote`/`DaxExpression` (string, or object with `Name`,
/// `Note` or `Expression`).
fn text(v: Option<&Value>) -> Option<String> {
    let v = v?;
    let s = match v {
        Value::String(s) => Some(s.as_str()),
        Value::Object(map) => ["Name", "Note", "Expression"]
            .iter()
            .find_map(|k| map.get(*k).and_then(Value::as_str)),
        _ => None,
    }?;
    (!s.is_empty()).then(|| s.to_owned())
}

fn num(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n
            .as_u64()
            .or_else(|| n.as_f64().filter(|f| *f > 0.0).map(|f| f as u64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

fn flag(v: Option<&Value>) -> bool {
    v.and_then(Value::as_bool).unwrap_or(false)
}

fn int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

fn data_type(name: Option<&str>) -> DataType {
    match name {
        Some(n) if n.eq_ignore_ascii_case("unknown") => DataType::Automatic,
        Some(n) => DataType::parse(n),
        None => DataType::Automatic,
    }
}

fn partition_mode(v: Option<&Value>) -> PartitionMode {
    match v {
        Some(Value::Number(n)) => match n.as_i64() {
            Some(0) => PartitionMode::Import,
            Some(1) => PartitionMode::DirectQuery,
            Some(4) => PartitionMode::Dual,
            Some(5) => PartitionMode::DirectLake,
            _ => PartitionMode::Default,
        },
        Some(Value::String(s)) => PartitionMode::parse(s),
        _ => PartitionMode::Default,
    }
}

fn partition_kind(v: Option<&Value>) -> String {
    match v {
        Some(Value::Number(n)) => match n.as_i64() {
            Some(1) => "query",
            Some(2) => "calculated",
            Some(3) => "none",
            Some(4) => "m",
            Some(5) => "entity",
            Some(6) => "policyRange",
            Some(7) => "calculationGroup",
            Some(8) => "inferred",
            _ => "unknown",
        }
        .to_owned(),
        Some(Value::String(s)) => {
            let mut c = s.chars();
            c.next()
                .map(|f| f.to_lowercase().chain(c).collect())
                .unwrap_or_default()
        }
        _ => "unknown".to_owned(),
    }
}

fn cross_filter(v: Option<&str>) -> CrossFilter {
    v.map_or(CrossFilter::OneDirection, CrossFilter::parse)
}

fn cardinality(v: Option<&str>, default: Cardinality) -> Cardinality {
    match v {
        Some(s) if s.eq_ignore_ascii_case("one") => Cardinality::One,
        Some(s) if s.eq_ignore_ascii_case("many") => Cardinality::Many,
        _ => default,
    }
}

/// `DaxModel.json` → statistics and (if needed) a model.
struct DaxModel<'a> {
    root: &'a Value,
    refs: Refs<'a>,
    /// Column object `$id` → (table, column).
    columns: HashMap<&'a str, (String, String)>,
}

impl<'a> DaxModel<'a> {
    fn new(root: &'a Value) -> Self {
        let refs = Refs::new(root);
        let mut columns = HashMap::new();
        for table in refs.list(root.get("Tables")) {
            let Some(table_name) = text(table.get("TableName")) else {
                continue;
            };
            for column in refs.list(table.get("Columns")) {
                if let (Some(Value::String(id)), Some(name)) =
                    (column.get("$id"), text(column.get("ColumnName")))
                {
                    columns.insert(id.as_str(), (table_name.clone(), name));
                }
            }
        }
        Self {
            root,
            refs,
            columns,
        }
    }

    fn column_ref(&self, v: Option<&'a Value>) -> Option<&(String, String)> {
        let column = self.refs.get(v?);
        let id = column.get("$id").and_then(Value::as_str)?;
        self.columns.get(id)
    }

    fn relationships(&self) -> Vec<VpaxRelationshipStats> {
        self.refs
            .list(self.root.get("Relationships"))
            .filter_map(|r| {
                let (from_table, from_column) = self.column_ref(r.get("FromColumn"))?.clone();
                let (to_table, to_column) = self.column_ref(r.get("ToColumn"))?.clone();
                Some(VpaxRelationshipStats {
                    from_table,
                    from_column,
                    to_table,
                    to_column,
                    used_size: num(r.get("UsedSizeFrom")) + num(r.get("UsedSizeTo")),
                    missing_keys: num(r.get("MissingKeys")),
                    invalid_rows: num(r.get("InvalidRows")),
                })
            })
            .collect()
    }

    fn stats(&self) -> VpaxStats {
        let root = self.root;
        let relationships = self.relationships();
        // Relationship sizes per table (`UsedSizeFrom` on the from table,
        // `UsedSizeTo` on the to table).
        let mut rel_size: HashMap<String, u64> = HashMap::new();
        for r in self.refs.list(root.get("Relationships")) {
            if let Some((t, _)) = self.column_ref(r.get("FromColumn")) {
                *rel_size.entry(t.to_lowercase()).or_default() += num(r.get("UsedSizeFrom"));
            }
            if let Some((t, _)) = self.column_ref(r.get("ToColumn")) {
                *rel_size.entry(t.to_lowercase()).or_default() += num(r.get("UsedSizeTo"));
            }
        }
        let mut tables = Vec::new();
        for table in self.refs.list(root.get("Tables")) {
            let Some(name) = text(table.get("TableName")) else {
                continue;
            };
            let columns: Vec<VpaxColumnStats> = self
                .refs
                .list(table.get("Columns"))
                .filter_map(|c| self.column_stats(c))
                .collect();
            let columns_size = columns.iter().map(|c| c.total_size).sum();
            let user_hierarchies_size = self
                .refs
                .list(table.get("UserHierarchies"))
                .map(|h| num(h.get("UsedSize")))
                .sum();
            let relationships_size = rel_size.get(&name.to_lowercase()).copied().unwrap_or(0);
            tables.push(VpaxTableStats {
                rows_count: num(table.get("RowsCount")),
                columns_size,
                user_hierarchies_size,
                relationships_size,
                table_size: columns_size + user_hierarchies_size + relationships_size,
                is_hidden: flag(table.get("IsHidden")),
                columns,
                name,
            });
        }
        let extractor = match (
            text(root.get("ExtractorApp")),
            text(root.get("ExtractorAppVersion")),
        ) {
            (Some(app), Some(v)) => Some(format!("{app} {v}")),
            (Some(app), None) => Some(app),
            _ => text(root.get("ExtractorLib")),
        };
        VpaxStats {
            source: "DaxModel.json".into(),
            model_name: text(root.get("ModelName")),
            compatibility_level: int(root.get("CompatibilityLevel")),
            extractor,
            extraction_date: text(root.get("ExtractionDate")),
            last_data_refresh: text(root.get("LastDataRefresh")),
            total_size: tables.iter().map(|t| t.table_size).sum(),
            tables,
            relationships,
        }
    }

    fn column_stats(&self, c: &'a Value) -> Option<VpaxColumnStats> {
        let name = text(c.get("ColumnName"))?;
        let segments: Vec<&Value> = self.refs.list(c.get("ColumnSegments")).collect();
        let data_size = if c.get("DataSize").is_some() {
            num(c.get("DataSize"))
        } else {
            segments.iter().map(|s| num(s.get("UsedSize"))).sum()
        };
        let hierarchies_size = if c.get("HierarchiesSize").is_some() {
            num(c.get("HierarchiesSize"))
        } else {
            self.refs
                .list(c.get("ColumnHierarchies"))
                .map(|h| num(h.get("UsedSize")))
                .sum()
        };
        let dictionary_size = num(c.get("DictionarySize"));
        Some(VpaxColumnStats {
            name,
            data_type: text(c.get("DataType")),
            encoding: text(c.get("Encoding")),
            cardinality: num(c.get("ColumnCardinality")),
            data_size,
            dictionary_size,
            hierarchies_size,
            total_size: data_size + dictionary_size + hierarchies_size,
            segments: segments.len() as u64,
            is_hidden: flag(c.get("IsHidden")),
        })
    }

    /// The model as far as `DaxModel.json` describes it.
    fn model(&self) -> Model {
        let root = self.root;
        let mut model = Model {
            name: text(root.get("ModelName")).unwrap_or_else(|| "Model".to_owned()),
            culture: text(root.get("Culture")),
            ..Model::default()
        };
        if let Some(level) = int(root.get("CompatibilityLevel")).filter(|l| *l > 0) {
            model.database = Some(Database {
                name: text(root.get("ModelName")),
                compatibility_level: Some(level),
                compatibility_mode: text(root.get("CompatibilityMode")),
                ..Database::default()
            });
        }
        for t in self.refs.list(root.get("Tables")) {
            let Some(name) = text(t.get("TableName")) else {
                continue;
            };
            let mut table = Table {
                name,
                description: text(t.get("Description")),
                is_hidden: flag(t.get("IsHidden")),
                is_private: flag(t.get("IsPrivate")),
                ..Table::default()
            };
            for c in self.refs.list(t.get("Columns")) {
                let Some(name) = text(c.get("ColumnName")) else {
                    continue;
                };
                let source = text(c.get("SourceColumn")).unwrap_or_else(|| name.clone());
                let kind = match c.get("ColumnType").and_then(Value::as_str) {
                    Some("RowNumber") => continue,
                    Some("Calculated") => ColumnKind::Calculated {
                        expression: text(c.get("ColumnExpression")).unwrap_or_default(),
                    },
                    Some("CalculatedTableColumn") => ColumnKind::CalculatedTableColumn {
                        source_column: source,
                    },
                    _ => ColumnKind::Data {
                        source_column: source,
                    },
                };
                table.columns.push(Column {
                    name,
                    description: text(c.get("Description")),
                    data_type: data_type(c.get("DataType").and_then(Value::as_str)),
                    kind,
                    is_hidden: flag(c.get("IsHidden")),
                    is_key: flag(c.get("IsKey")),
                    format_string: text(c.get("FormatString")),
                    sort_by_column: text(c.get("SortByColumnName")),
                    display_folder: text(c.get("DisplayFolder")),
                    ..Column::default()
                });
            }
            for m in self.refs.list(t.get("Measures")) {
                let Some(name) = text(m.get("MeasureName")) else {
                    continue;
                };
                let mut measure =
                    Measure::new(name, text(m.get("MeasureExpression")).unwrap_or_default());
                measure.format_string = text(m.get("FormatString"));
                measure.display_folder = text(m.get("DisplayFolder"));
                measure.description = text(m.get("Description"));
                measure.is_hidden = flag(m.get("IsHidden"));
                table.measures.push(measure);
            }
            for h in self.refs.list(t.get("UserHierarchies")) {
                let Some(name) = text(h.get("HierarchyName")) else {
                    continue;
                };
                let levels = self
                    .refs
                    .list(h.get("Levels"))
                    .filter_map(|c| text(c.get("ColumnName")))
                    .enumerate()
                    .map(|(i, column)| Level {
                        name: column.clone(),
                        ordinal: i64::try_from(i).ok(),
                        column,
                        ..Level::default()
                    })
                    .collect();
                table.hierarchies.push(Hierarchy {
                    name,
                    is_hidden: flag(h.get("IsHidden")),
                    levels,
                    ..Hierarchy::default()
                });
            }
            let table_expression = text(t.get("TableExpression"));
            for p in self.refs.list(t.get("Partitions")) {
                let kind = partition_kind(p.get("Type"));
                let source = match (&table_expression, kind.as_str()) {
                    (Some(expression), "calculated") => PartitionSource::Calculated {
                        expression: expression.clone(),
                    },
                    _ => PartitionSource::Unknown {
                        kind,
                        props: Default::default(),
                    },
                };
                table.partitions.push(Partition {
                    name: text(p.get("PartitionName")).unwrap_or_else(|| table.name.clone()),
                    mode: partition_mode(p.get("Mode")),
                    source,
                    description: text(p.get("Description")),
                    ..Partition::default()
                });
            }
            if table.partitions.is_empty()
                && let Some(expression) = table_expression
            {
                table.partitions.push(Partition {
                    name: table.name.clone(),
                    source: PartitionSource::Calculated { expression },
                    ..Partition::default()
                });
            }
            model.tables.push(table);
        }
        for r in self.refs.list(root.get("Relationships")) {
            let (Some(from), Some(to)) = (
                self.column_ref(r.get("FromColumn")),
                self.column_ref(r.get("ToColumn")),
            ) else {
                continue;
            };
            model.relationships.push(Relationship {
                name: text(r.get("Name")).unwrap_or_default(),
                from: ColumnRef::new(&from.0, &from.1),
                to: ColumnRef::new(&to.0, &to.1),
                from_cardinality: cardinality(
                    r.get("FromCardinalityType").and_then(Value::as_str),
                    Cardinality::Many,
                ),
                to_cardinality: cardinality(
                    r.get("ToCardinalityType").and_then(Value::as_str),
                    Cardinality::One,
                ),
                cross_filtering_behavior: cross_filter(
                    r.get("CrossFilteringBehavior").and_then(Value::as_str),
                ),
                is_active: r.get("IsActive").and_then(Value::as_bool).unwrap_or(false),
                rely_on_referential_integrity: flag(r.get("RelyOnReferentialIntegrity")),
                ..Relationship::default()
            });
        }
        model
    }
}

/// `DaxVpaView.json` → statistics (flat lists keyed by table/column name).
fn view_stats(root: &Value) -> VpaxStats {
    let list = |key: &str| {
        root.get(key)
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice)
    };
    let mut segments: HashMap<(String, String), u64> = HashMap::new();
    for s in list("ColumnsSegments") {
        if let (Some(t), Some(c)) = (text(s.get("TableName")), text(s.get("ColumnName"))) {
            *segments
                .entry((t.to_lowercase(), c.to_lowercase()))
                .or_default() += 1;
        }
    }
    let mut columns: HashMap<String, Vec<VpaxColumnStats>> = HashMap::new();
    for c in list("Columns") {
        let (Some(table), Some(name)) = (text(c.get("TableName")), text(c.get("ColumnName")))
        else {
            continue;
        };
        let key = (table.to_lowercase(), name.to_lowercase());
        let data_size = num(c.get("DataSize"));
        let dictionary_size = num(c.get("DictionarySize"));
        let hierarchies_size = num(c.get("HierarchiesSize"));
        let total = num(c.get("TotalSize"));
        columns
            .entry(table.to_lowercase())
            .or_default()
            .push(VpaxColumnStats {
                name,
                data_type: text(c.get("DataType")),
                encoding: text(c.get("Encoding")),
                cardinality: num(c.get("ColumnCardinality")),
                data_size,
                dictionary_size,
                hierarchies_size,
                total_size: if total > 0 {
                    total
                } else {
                    data_size + dictionary_size + hierarchies_size
                },
                segments: segments.get(&key).copied().unwrap_or(0),
                is_hidden: flag(c.get("IsHidden")),
            });
    }
    let tables: Vec<VpaxTableStats> = list("Tables")
        .iter()
        .filter_map(|t| {
            let name = text(t.get("TableName"))?;
            let columns = columns.remove(&name.to_lowercase()).unwrap_or_default();
            let columns_size = match num(t.get("ColumnsSize")) {
                0 => columns.iter().map(|c| c.total_size).sum(),
                n => n,
            };
            let user_hierarchies_size = num(t.get("UserHierarchiesSize"));
            let relationships_size = num(t.get("RelationshipsSize"));
            let table_size = match num(t.get("TableSize")) {
                0 => columns_size + user_hierarchies_size + relationships_size,
                n => n,
            };
            Some(VpaxTableStats {
                name,
                rows_count: num(t.get("RowsCount")),
                columns_size,
                user_hierarchies_size,
                relationships_size,
                table_size,
                is_hidden: flag(t.get("IsHidden")),
                columns,
            })
        })
        .collect();
    let relationships = list("Relationships")
        .iter()
        .filter_map(|r| {
            Some(VpaxRelationshipStats {
                from_table: text(r.get("FromTableName"))?,
                from_column: text(r.get("FromColumnName"))?,
                to_table: text(r.get("ToTableName"))?,
                to_column: text(r.get("ToColumnName"))?,
                used_size: num(r.get("UsedSize")),
                missing_keys: num(r.get("MissingKeys")),
                invalid_rows: num(r.get("InvalidRows")),
            })
        })
        .collect();
    VpaxStats {
        source: "DaxVpaView.json".into(),
        model_name: text(root.get("ModelName")),
        total_size: tables.iter().map(|t| t.table_size).sum(),
        tables,
        relationships,
        ..VpaxStats::default()
    }
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(crate) fn read(b: &mut Builder<'_>) {
    b.doc.source.format = SourceFormat::Vpax;
    let container = Arc::clone(&b.container);

    if let Some(entry) = root_file(container.as_ref(), "Model.bim")
        && let Some(loaded) = load_tmsl(&container, "", &entry, &mut b.doc.diagnostics)
    {
        b.consumed.extend(loaded.consumed);
        b.doc.model = Some(loaded.model);
        b.model_source = Some(loaded.source);
    }

    let mut stats = None;
    if let Some(entry) = root_file(container.as_ref(), "DaxModel.json") {
        match read_json_capped(container.as_ref(), &entry, VPAX_JSON_MAX) {
            Ok((json, _)) if json.is_object() => {
                let dax = DaxModel::new(&json);
                stats = Some(dax.stats());
                if b.doc.model.is_none() {
                    let mut model = dax.model();
                    model.origin = Some(Origin::file(entry.clone()));
                    b.doc.model = Some(model);
                    b.diag(
                        Diagnostic::info(
                            codes::VPAX,
                            "no Model.bim in the export: the model was rebuilt from DaxModel.json (names, types and DAX; no M queries)",
                        )
                        .with_origin(Origin::file(entry.clone())),
                    );
                }
                b.consumed.insert(entry);
            }
            Ok(_) => b.diag(
                Diagnostic::warning(codes::VPAX, format!("{entry}: not a JSON object"))
                    .with_origin(Origin::file(entry)),
            ),
            Err(e) => b.diag(
                Diagnostic::warning(codes::VPAX, format!("{entry}: {e}"))
                    .with_origin(Origin::file(entry)),
            ),
        }
    }
    if let Some(entry) = root_file(container.as_ref(), "DaxVpaView.json") {
        if stats.is_none() {
            match read_json_capped(container.as_ref(), &entry, VPAX_JSON_MAX) {
                Ok((json, _)) => {
                    stats = Some(view_stats(&json));
                    b.consumed.insert(entry);
                }
                Err(e) => b.diag(
                    Diagnostic::warning(codes::VPAX, format!("{entry}: {e}"))
                        .with_origin(Origin::file(entry)),
                ),
            }
        } else {
            b.consumed.insert(entry);
        }
    }
    if let Some(entry) = root_file(container.as_ref(), "CompatabilityMode.txt")
        && let Ok((mode, _)) = read_text_capped(container.as_ref(), &entry, 1024)
    {
        let mode = mode.trim().to_owned();
        if let Some(db) = b.doc.model.as_mut().and_then(|m| m.database.as_mut())
            && db.compatibility_mode.is_none()
            && !mode.is_empty()
        {
            db.compatibility_mode = Some(mode);
        }
        b.consumed.insert(entry);
    }

    match stats {
        Some(stats) => {
            let columns: usize = stats.tables.iter().map(|t| t.columns.len()).sum();
            b.diag(Diagnostic::info(
                codes::VPAX,
                format!(
                    "VertiPaq Analyzer statistics from {}: {} tables, {columns} columns, total size {}",
                    stats.source,
                    stats.tables.len(),
                    human_size(stats.total_size)
                ),
            ));
            b.outline = Some(FormatOutline::Vpax(stats));
        }
        None => b.diag(Diagnostic::warning(
            codes::VPAX,
            "the export has no readable DaxModel.json or DaxVpaView.json: no statistics",
        )),
    }
    if b.doc.model.is_none() {
        b.diag(Diagnostic::info(
            codes::NO_MODEL,
            "the export has no readable Model.bim or DaxModel.json: no model",
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_resolve_ids_values_and_names() {
        let json: Value = serde_json::json!({
            "$id": "1",
            "Tables": {"$id": "2", "$values": [
                {"$id": "3", "TableName": {"Name": "T"}, "Model": {"$ref": "1"},
                 "Columns": {"$id": "4", "$values": [{"$id": "5", "ColumnName": "C"}]}}
            ]},
            "Relationships": [{"FromColumn": {"$ref": "5"}, "ToColumn": {"$ref": "5"}}]
        });
        let dax = DaxModel::new(&json);
        assert_eq!(
            dax.column_ref(json["Relationships"][0].get("FromColumn")),
            Some(&("T".to_owned(), "C".to_owned()))
        );
        assert_eq!(
            text(Some(&serde_json::json!({"Note": "n"}))).as_deref(),
            Some("n")
        );
        assert_eq!(num(Some(&serde_json::json!(-3))), 0);
        assert_eq!(human_size(1536), "1.5 KiB");
    }
}
