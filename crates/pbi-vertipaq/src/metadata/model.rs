//! Maps the TOM metadata tables of `metadata.sqlitedb` to [`pbi_model::Model`].
//!
//! Typed fields are filled where the model has them; other TOM properties that TMDL
//! knows go to `extra` under their TMDL names (`showAsVariationsOnly`,
//! `isAvailableInMdx`, `variation`, `joinOnDateBehavior`, `calculationGroup`, ...).
//! Facts derived from storage use a `vertipaq.` prefix and are not TMDL properties:
//!
//! * `vertipaq.systemFlags` (tables): raw `Table.SystemFlags` when non-zero
//!   (bit 0 = internal storage table, 2 = calculated table);
//! * `vertipaq.internalTable` (tables): `true` for `H$`/`R$`/`U$` tables (only present
//!   with [`ModelOptions::include_system_tables`]);
//! * `vertipaq.autoDateTable` (tables): `local` for `LocalDateTable_*`, `template` for
//!   `DateTableTemplate_*` (auto date/time tables Power BI generates).

use std::collections::HashMap;

use pbi_model::{
    Annotation, Cardinality, Column, ColumnKind, ColumnRef, Culture, ExpressionKind, Hierarchy,
    Level, Measure, Model, NamedExpression, Origin, Partition, PartitionSource, Perspective,
    PropValue, PropertyBag, QueryGroup, RawObject, Relationship, Role, Table, TablePermission,
    tmdl_name,
};

use super::ModelOptions;
use super::codes::{self, ColumnKindCode, object_type};
use super::rows::{Row, Rows};

/// The TOM metadata tables needed for the model.
pub(crate) struct Catalog {
    pub model: Rows,
    pub tables: Rows,
    pub columns: Rows,
    pub measures: Rows,
    pub partitions: Rows,
    pub relationships: Rows,
    pub expressions: Rows,
    pub hierarchies: Rows,
    pub levels: Rows,
    pub annotations: Rows,
    pub query_groups: Rows,
    pub data_sources: Rows,
    pub roles: Rows,
    pub table_permissions: Rows,
    pub cultures: Rows,
    pub linguistic_metadata: Rows,
    pub perspectives: Rows,
    pub variations: Rows,
    pub calculation_groups: Rows,
    pub calculation_items: Rows,
    pub format_string_definitions: Rows,
    pub detail_rows_definitions: Rows,
    pub kpis: Rows,
}

fn json_number(value: i64) -> PropValue {
    PropValue::Json(serde_json::Value::from(value))
}

struct Mapper<'a> {
    cat: &'a Catalog,
    opts: &'a ModelOptions,
    annotations: HashMap<(i64, i64), Vec<Annotation>>,
    /// Table ID → (name, included in the model).
    table_names: HashMap<i64, (String, bool)>,
    /// Column ID → (table ID, name).
    column_names: HashMap<i64, (i64, String)>,
}

fn column_name(row: Row<'_>) -> String {
    row.string("ExplicitName")
        .or_else(|| row.string("InferredName"))
        .unwrap_or_default()
}

fn column_kind_code(row: Row<'_>) -> ColumnKindCode {
    ColumnKindCode::from_code(
        row.i64("Type")
            .or_else(|| row.i64("BindingType"))
            .unwrap_or(1),
    )
}

fn auto_date_kind(name: &str, annotations: &[Annotation]) -> Option<&'static str> {
    let flagged = |key: &str| {
        annotations
            .iter()
            .any(|a| a.name == key && a.value.trim().eq_ignore_ascii_case("true"))
    };
    if flagged("__PBI_LocalDateTable") || name.starts_with("LocalDateTable_") {
        Some("local")
    } else if flagged("__PBI_TemplateDateTable") || name.starts_with("DateTableTemplate_") {
        Some("template")
    } else {
        None
    }
}

fn group_by<'a>(rows: &'a Rows, column: &str) -> HashMap<i64, Vec<Row<'a>>> {
    let mut map: HashMap<i64, Vec<Row<'a>>> = HashMap::new();
    for row in rows.iter() {
        if let Some(key) = row.i64(column) {
            map.entry(key).or_default().push(row);
        }
    }
    map
}

impl<'a> Mapper<'a> {
    fn new(cat: &'a Catalog, opts: &'a ModelOptions) -> Self {
        let mut annotations: HashMap<(i64, i64), Vec<Annotation>> = HashMap::new();
        for row in cat.annotations.iter() {
            let (Some(kind), Some(object)) = (row.i64("ObjectType"), row.i64("ObjectID")) else {
                continue;
            };
            let Some(name) = row.string("Name") else {
                continue;
            };
            let value = row.raw_str("Value").unwrap_or_default();
            annotations
                .entry((kind, object))
                .or_default()
                .push(Annotation::new(name, value));
        }
        let table_names = cat
            .tables
            .iter()
            .map(|t| {
                let internal = t.i64("SystemFlags").unwrap_or(0) & 1 != 0;
                (
                    t.id(),
                    (
                        t.string("Name").unwrap_or_default(),
                        !internal || opts.include_system_tables,
                    ),
                )
            })
            .collect();
        let column_names = cat
            .columns
            .iter()
            .map(|c| (c.id(), (c.i64("TableID").unwrap_or(0), column_name(c))))
            .collect();
        Self {
            cat,
            opts,
            annotations,
            table_names,
            column_names,
        }
    }

    fn annotations(&self, kind: i64, id: i64) -> Vec<Annotation> {
        self.annotations
            .get(&(kind, id))
            .cloned()
            .unwrap_or_default()
    }

    fn origin(&self, table: &str, id: i64) -> Option<Origin> {
        self.opts
            .origin_path
            .as_ref()
            .map(|path| Origin::file(path.clone()).with_pointer(format!("/{table}/{id}")))
    }

    fn table_name(&self, id: Option<i64>) -> Option<&str> {
        self.table_names.get(&id?).map(|(name, _)| name.as_str())
    }

    fn table_included(&self, id: Option<i64>) -> bool {
        id.and_then(|id| self.table_names.get(&id))
            .is_some_and(|(_, included)| *included)
    }

    fn column_ref(&self, column_id: Option<i64>) -> Option<ColumnRef> {
        let (table_id, name) = self.column_names.get(&column_id?)?;
        Some(ColumnRef::new(
            self.table_name(Some(*table_id))?,
            name.clone(),
        ))
    }

    fn query_group_folder(&self, id: Option<i64>) -> Option<String> {
        self.cat
            .query_groups
            .by_id(id.filter(|&i| i > 0)?)?
            .string("Folder")
    }

    fn expression_text(rows: &Rows, id: Option<i64>) -> Option<String> {
        rows.by_id(id.filter(|&i| i > 0)?)?.string("Expression")
    }

    fn build(&self) -> Model {
        let cat = self.cat;
        let mut model = Model {
            name: "Model".into(),
            ..Model::default()
        };
        if let Some(row) = cat.model.iter().next() {
            let id = row.id();
            if let Some(name) = row.string("Name") {
                model.name = name;
            }
            model.description = row.string("Description");
            model.culture = row.string("Culture");
            model.default_power_bi_data_source_version = row
                .i64("DefaultPowerBIDataSourceVersion")
                .and_then(codes::power_bi_data_source_version);
            model.source_query_culture = row.string("SourceQueryCulture");
            model.discourage_implicit_measures = row.flag("DiscourageImplicitMeasures");
            if let Some(options) = row.str("DataAccessOptions") {
                let value = serde_json::from_str::<serde_json::Value>(options)
                    .map(PropValue::Json)
                    .unwrap_or_else(|_| PropValue::Text(options.to_owned()));
                model.extra.insert("dataAccessOptions".into(), value);
            }
            if let Some(mode) = row.i64("DefaultMode").filter(|&m| m != 0) {
                model.extra.insert(
                    "defaultMode".into(),
                    PropValue::Text(codes::partition_mode(mode).as_str().to_owned()),
                );
            }
            if let Some(collation) = row.string("Collation") {
                model
                    .extra
                    .insert("collation".into(), PropValue::Text(collation));
            }
            if row.flag("DiscourageReportMeasures") {
                model
                    .extra
                    .insert("discourageReportMeasures".into(), PropValue::Bool(true));
            }
            model.annotations = self.annotations(object_type::MODEL, id);
            model.origin = self.origin("Model", id);
        }

        self.add_data_sources(&mut model);
        model.tables = self.tables();
        model.relationships = self.relationships();
        model.expressions = self.expressions();
        model.query_groups = cat
            .query_groups
            .iter()
            .map(|row| QueryGroup {
                folder: row.string("Folder").unwrap_or_default(),
                description: row.string("Description"),
                annotations: self.annotations(object_type::QUERY_GROUP, row.id()),
                ..QueryGroup::default()
            })
            .collect();
        model.roles = self.roles();
        model.cultures = self.cultures();
        model.perspectives = cat
            .perspectives
            .iter()
            .map(|row| Perspective {
                name: row.string("Name").unwrap_or_default(),
                description: row.string("Description"),
                annotations: self.annotations(object_type::PERSPECTIVE, row.id()),
                ..Perspective::default()
            })
            .collect();
        model
    }

    fn add_data_sources(&self, model: &mut Model) {
        // Connection strings and credentials are not copied (they may hold secrets).
        let sources: Vec<PropValue> = self
            .cat
            .data_sources
            .iter()
            .map(|row| {
                let mut object = RawObject {
                    name: row.string("Name"),
                    description: row.string("Description"),
                    ..RawObject::default()
                };
                if let Some(kind) = row.i64("Type") {
                    let kind = match kind {
                        1 => "provider".to_owned(),
                        2 => "structured".to_owned(),
                        other => format!("type{other}"),
                    };
                    object
                        .properties
                        .insert("type".into(), PropValue::Text(kind));
                }
                if let Some(provider) = row.string("Provider") {
                    object
                        .properties
                        .insert("provider".into(), PropValue::Text(provider));
                }
                PropValue::Object(Box::new(object))
            })
            .collect();
        if !sources.is_empty() {
            model
                .extra
                .insert("dataSources".into(), PropValue::List(sources));
        }
    }

    fn tables(&self) -> Vec<Table> {
        let cat = self.cat;
        let columns_by_table = group_by(&cat.columns, "TableID");
        let measures_by_table = group_by(&cat.measures, "TableID");
        let partitions_by_table = group_by(&cat.partitions, "TableID");
        let hierarchies_by_table = group_by(&cat.hierarchies, "TableID");
        let levels_by_hierarchy = group_by(&cat.levels, "HierarchyID");
        let variations_by_column = group_by(&cat.variations, "ColumnID");

        let mut out = Vec::new();
        for row in cat.tables.iter() {
            let id = row.id();
            let system_flags = row.i64("SystemFlags").unwrap_or(0);
            let internal = system_flags & 1 != 0;
            if internal && !self.opts.include_system_tables {
                continue;
            }
            let mut table = Table::new(row.string("Name").unwrap_or_default());
            table.description = row.string("Description");
            table.lineage_tag = row.string("LineageTag");
            table.source_lineage_tag = row.string("SourceLineageTag");
            table.is_hidden = row.flag("IsHidden") || internal;
            table.is_private = row.flag("IsPrivate");
            table.data_category = row.string("DataCategory");
            table.annotations = self.annotations(object_type::TABLE, id);
            table.origin = self.origin("Table", id);
            for (key, column) in [
                ("showAsVariationsOnly", "ShowAsVariationsOnly"),
                ("excludeFromModelRefresh", "ExcludeFromModelRefresh"),
                ("systemManaged", "SystemManaged"),
                (
                    "excludeFromAutomaticAggregations",
                    "ExcludeFromAutomaticAggregations",
                ),
            ] {
                if row.flag(column) {
                    table.extra.insert(key.into(), PropValue::Bool(true));
                }
            }
            if let Some(group) = self.calculation_group(row.id_ref("CalculationGroupID"), id) {
                table.extra.insert("calculationGroup".into(), group);
            }
            if system_flags != 0 {
                table
                    .extra
                    .insert("vertipaq.systemFlags".into(), json_number(system_flags));
            }
            if internal {
                table
                    .extra
                    .insert("vertipaq.internalTable".into(), PropValue::Bool(true));
            }
            if let Some(kind) = auto_date_kind(&table.name, &table.annotations) {
                table.extra.insert(
                    "vertipaq.autoDateTable".into(),
                    PropValue::Text(kind.into()),
                );
            }

            for column in columns_by_table.get(&id).into_iter().flatten() {
                if let Some(c) = self.column(*column, &variations_by_column) {
                    table.columns.push(c);
                }
            }
            for measure in measures_by_table.get(&id).into_iter().flatten() {
                table.measures.push(self.measure(*measure));
            }
            for partition in partitions_by_table.get(&id).into_iter().flatten() {
                table.partitions.push(self.partition(*partition));
            }
            for hierarchy in hierarchies_by_table.get(&id).into_iter().flatten() {
                table
                    .hierarchies
                    .push(self.hierarchy(*hierarchy, &levels_by_hierarchy));
            }
            out.push(table);
        }
        out
    }

    fn column(
        &self,
        row: Row<'_>,
        variations_by_column: &HashMap<i64, Vec<Row<'_>>>,
    ) -> Option<Column> {
        let id = row.id();
        let kind_code = column_kind_code(row);
        if kind_code == ColumnKindCode::RowNumber && !self.opts.include_row_number_columns {
            return None;
        }
        let (code, inferred) =
            codes::effective_data_type(row.i64("ExplicitDataType"), row.i64("InferredDataType"));
        let source_column = row.string("SourceColumn").unwrap_or_default();
        let mut column = Column {
            name: column_name(row),
            data_type: codes::data_type(code),
            kind: match kind_code {
                ColumnKindCode::Calculated => ColumnKind::Calculated {
                    expression: row.raw_str("Expression").unwrap_or_default().to_owned(),
                },
                ColumnKindCode::RowNumber => ColumnKind::RowNumber,
                ColumnKindCode::CalculatedTableColumn => {
                    ColumnKind::CalculatedTableColumn { source_column }
                }
                ColumnKindCode::Data | ColumnKindCode::Other(_) => {
                    ColumnKind::Data { source_column }
                }
            },
            description: row.string("Description"),
            lineage_tag: row.string("LineageTag"),
            source_lineage_tag: row.string("SourceLineageTag"),
            is_hidden: row.flag("IsHidden"),
            is_key: row.flag("IsKey"),
            format_string: row.string("FormatString"),
            summarize_by: codes::summarize_by(row.i64("SummarizeBy")),
            sort_by_column: row
                .id_ref("SortByColumnID")
                .and_then(|id| self.column_names.get(&id))
                .map(|(_, name)| name.clone()),
            display_folder: row.string("DisplayFolder"),
            data_category: row.string("DataCategory"),
            annotations: self.annotations(object_type::COLUMN, id),
            origin: self.origin("Column", id),
            ..Column::default()
        };
        let calculated = matches!(
            kind_code,
            ColumnKindCode::Calculated | ColumnKindCode::CalculatedTableColumn
        );
        column.is_data_type_inferred = inferred && calculated;
        column.is_name_inferred =
            kind_code == ColumnKindCode::CalculatedTableColumn && row.str("ExplicitName").is_none();

        let extra = &mut column.extra;
        if let ColumnKindCode::Other(code) = kind_code {
            extra.insert("vertipaq.columnType".into(), json_number(code));
        }
        if row.opt_flag("IsAvailableInMDX") == Some(false) {
            extra.insert("isAvailableInMdx".into(), PropValue::Bool(false));
        }
        if row.opt_flag("IsNullable") == Some(false) {
            extra.insert("isNullable".into(), PropValue::Bool(false));
        }
        for (key, name) in [
            ("isUnique", "IsUnique"),
            ("isDefaultLabel", "IsDefaultLabel"),
            ("isDefaultImage", "IsDefaultImage"),
            ("keepUniqueRows", "KeepUniqueRows"),
        ] {
            if row.flag(name) {
                extra.insert(key.into(), PropValue::Bool(true));
            }
        }
        if let Some(hint) = row.i64("EncodingHint").and_then(codes::encoding_hint) {
            extra.insert("encodingHint".into(), PropValue::Text(hint.into()));
        }
        if let Some(alignment) = row.i64("Alignment").and_then(|a| match a {
            2 => Some("left"),
            3 => Some("right"),
            4 => Some("center"),
            _ => None,
        }) {
            extra.insert("alignment".into(), PropValue::Text(alignment.into()));
        }
        let variations: Vec<PropValue> = variations_by_column
            .get(&id)
            .into_iter()
            .flatten()
            .map(|v| self.variation(*v))
            .collect();
        match variations.len() {
            0 => {}
            1 => {
                if let Some(v) = variations.into_iter().next() {
                    extra.insert("variation".into(), v);
                }
            }
            _ => {
                extra.insert("variation".into(), PropValue::List(variations));
            }
        }
        Some(column)
    }

    fn variation(&self, row: Row<'_>) -> PropValue {
        let mut object = RawObject {
            name: row.string("Name"),
            description: row.string("Description"),
            ..RawObject::default()
        };
        if row.flag("IsDefault") {
            object
                .properties
                .insert("isDefault".into(), PropValue::Bool(true));
        }
        if let Some(name) = row
            .id_ref("RelationshipID")
            .and_then(|id| self.cat.relationships.by_id(id))
            .and_then(|r| r.string("Name"))
        {
            object
                .properties
                .insert("relationship".into(), PropValue::Text(name));
        }
        if let Some(hierarchy) = row
            .id_ref("DefaultHierarchyID")
            .and_then(|id| self.cat.hierarchies.by_id(id))
            && let (Some(table), Some(name)) = (
                self.table_name(hierarchy.i64("TableID")),
                hierarchy.str("Name"),
            )
        {
            object.properties.insert(
                "defaultHierarchy".into(),
                PropValue::Text(format!("{}.{}", tmdl_name(table), tmdl_name(name))),
            );
        }
        if let Some(column) = self.column_ref(row.id_ref("DefaultColumnID")) {
            object.properties.insert(
                "defaultColumn".into(),
                PropValue::Text(format!(
                    "{}.{}",
                    tmdl_name(&column.table),
                    tmdl_name(&column.column)
                )),
            );
        }
        PropValue::Object(Box::new(object))
    }

    fn measure(&self, row: Row<'_>) -> Measure {
        let id = row.id();
        let mut measure = Measure::new(
            row.string("Name").unwrap_or_default(),
            row.raw_str("Expression").unwrap_or_default(),
        );
        measure.format_string = row.string("FormatString");
        measure.display_folder = row.string("DisplayFolder");
        measure.is_hidden = row.flag("IsHidden");
        measure.description = row.string("Description");
        measure.lineage_tag = row.string("LineageTag");
        measure.data_category = row.string("DataCategory");
        measure.annotations = self.annotations(object_type::MEASURE, id);
        measure.origin = self.origin("Measure", id);
        if let Some(tag) = row.string("SourceLineageTag") {
            measure
                .extra
                .insert("sourceLineageTag".into(), PropValue::Text(tag));
        }
        if let Some(text) = Self::expression_text(
            &self.cat.format_string_definitions,
            row.i64("FormatStringDefinitionID"),
        ) {
            measure
                .extra
                .insert("formatStringDefinition".into(), PropValue::Expression(text));
        }
        if let Some(text) = Self::expression_text(
            &self.cat.detail_rows_definitions,
            row.i64("DetailRowsDefinitionID"),
        ) {
            measure
                .extra
                .insert("detailRowsDefinition".into(), PropValue::Expression(text));
        }
        if let Some(kpi) = row.id_ref("KPIID").and_then(|id| self.cat.kpis.by_id(id)) {
            let mut object = RawObject {
                description: kpi.string("Description"),
                ..RawObject::default()
            };
            for (key, column) in [
                ("targetExpression", "TargetExpression"),
                ("statusExpression", "StatusExpression"),
                ("trendExpression", "TrendExpression"),
            ] {
                if let Some(text) = kpi.string(column) {
                    object
                        .properties
                        .insert(key.into(), PropValue::Expression(text));
                }
            }
            for (key, column) in [
                ("targetFormatString", "TargetFormatString"),
                ("statusGraphic", "StatusGraphic"),
                ("trendGraphic", "TrendGraphic"),
            ] {
                if let Some(text) = kpi.string(column) {
                    object.properties.insert(key.into(), PropValue::Text(text));
                }
            }
            measure
                .extra
                .insert("kpi".into(), PropValue::Object(Box::new(object)));
        }
        measure
    }

    fn partition(&self, row: Row<'_>) -> Partition {
        let id = row.id();
        let text = row
            .raw_str("QueryDefinition")
            .unwrap_or_default()
            .to_owned();
        let kind = row
            .i64("Type")
            .map(codes::partition_source_kind)
            .unwrap_or("unknown");
        let source = match kind {
            "m" => PartitionSource::M { expression: text },
            "calculated" => PartitionSource::Calculated { expression: text },
            "query" => PartitionSource::Query {
                query: text,
                data_source: row
                    .id_ref("DataSourceID")
                    .and_then(|id| self.cat.data_sources.by_id(id))
                    .and_then(|ds| ds.string("Name")),
            },
            "entity" => PartitionSource::Entity {
                entity_name: text,
                expression_source: row
                    .id_ref("ExpressionSourceID")
                    .and_then(|id| self.cat.expressions.by_id(id))
                    .and_then(|e| e.string("Name")),
                schema_name: row.string("SchemaName"),
            },
            other => {
                let mut props = PropertyBag::new();
                if !text.is_empty() {
                    props.insert("queryDefinition".into(), PropValue::Expression(text));
                }
                if other == "unknown"
                    && let Some(code) = row.i64("Type")
                {
                    props.insert("vertipaq.sourceType".into(), json_number(code));
                }
                if other == "policyRange" {
                    for (key, column) in [("start", "RangeStart"), ("end", "RangeEnd")] {
                        if let Some(value) = row.f64(column) {
                            props.insert(
                                key.into(),
                                PropValue::Json(serde_json::Value::from(value)),
                            );
                        }
                    }
                    if let Some(granularity) = row.i64("RangeGranularity") {
                        props.insert("granularity".into(), json_number(granularity));
                    }
                }
                PartitionSource::Unknown {
                    kind: other.to_owned(),
                    props,
                }
            }
        };
        let mut partition = Partition {
            name: row.string("Name").unwrap_or_default(),
            mode: codes::partition_mode(row.i64("Mode").unwrap_or(2)),
            source,
            query_group: self.query_group_folder(row.i64("QueryGroupID")),
            description: row.string("Description"),
            annotations: self.annotations(object_type::PARTITION, id),
            origin: self.origin("Partition", id),
            ..Partition::default()
        };
        if let Some(view) = row
            .i64("DataView")
            .filter(|&v| v != 3)
            .and_then(codes::data_view)
        {
            partition
                .extra
                .insert("dataView".into(), PropValue::Text(view.into()));
        }
        partition
    }

    fn hierarchy(
        &self,
        row: Row<'_>,
        levels_by_hierarchy: &HashMap<i64, Vec<Row<'_>>>,
    ) -> Hierarchy {
        let id = row.id();
        let mut levels: Vec<Level> = levels_by_hierarchy
            .get(&id)
            .into_iter()
            .flatten()
            .map(|level| Level {
                name: level.string("Name").unwrap_or_default(),
                ordinal: level.i64("Ordinal").or_else(|| level.i64("Index")),
                column: level
                    .id_ref("ColumnID")
                    .and_then(|id| self.column_names.get(&id))
                    .map(|(_, name)| name.clone())
                    .unwrap_or_default(),
                description: level.string("Description"),
                lineage_tag: level.string("LineageTag"),
                annotations: self.annotations(object_type::LEVEL, level.id()),
                ..Level::default()
            })
            .collect();
        levels.sort_by_key(|l| l.ordinal.unwrap_or(i64::MAX));
        let mut hierarchy = Hierarchy {
            name: row.string("Name").unwrap_or_default(),
            description: row.string("Description"),
            lineage_tag: row.string("LineageTag"),
            is_hidden: row.flag("IsHidden"),
            display_folder: row.string("DisplayFolder"),
            levels,
            annotations: self.annotations(object_type::HIERARCHY, id),
            origin: self.origin("Hierarchy", id),
            ..Hierarchy::default()
        };
        if row.i64("HideMembers") == Some(1) {
            hierarchy.extra.insert(
                "hideMembers".into(),
                PropValue::Text("hideBlankMembers".into()),
            );
        }
        hierarchy
    }

    fn calculation_group(&self, id: Option<i64>, table_id: i64) -> Option<PropValue> {
        let group = match id {
            Some(id) => self.cat.calculation_groups.by_id(id),
            None => self
                .cat
                .calculation_groups
                .iter()
                .find(|g| g.i64("TableID") == Some(table_id)),
        }?;
        let mut object = RawObject {
            description: group.string("Description"),
            ..RawObject::default()
        };
        if let Some(precedence) = group.i64("Precedence") {
            object
                .properties
                .insert("precedence".into(), json_number(precedence));
        }
        let mut items: Vec<(i64, PropValue)> = self
            .cat
            .calculation_items
            .iter()
            .filter(|item| item.i64("CalculationGroupID") == Some(group.id()))
            .map(|item| {
                let mut object = RawObject {
                    name: item.string("Name"),
                    expression: item.string("Expression"),
                    description: item.string("Description"),
                    ..RawObject::default()
                };
                if let Some(ordinal) = item.i64("Ordinal") {
                    object
                        .properties
                        .insert("ordinal".into(), json_number(ordinal));
                }
                if let Some(text) = Self::expression_text(
                    &self.cat.format_string_definitions,
                    item.i64("FormatStringDefinitionID"),
                ) {
                    object
                        .properties
                        .insert("formatStringDefinition".into(), PropValue::Expression(text));
                }
                (
                    item.i64("Ordinal").unwrap_or(i64::MAX),
                    PropValue::Object(Box::new(object)),
                )
            })
            .collect();
        items.sort_by_key(|(ordinal, _)| *ordinal);
        if !items.is_empty() {
            object.properties.insert(
                "calculationItem".into(),
                PropValue::List(items.into_iter().map(|(_, v)| v).collect()),
            );
        }
        Some(PropValue::Object(Box::new(object)))
    }

    /// `FromX`/`ToX` value, falling back to the legacy `FromEndX`/`ToEndX` column.
    fn end_value(row: Row<'_>, column: &str) -> Option<i64> {
        row.i64(column).or_else(|| {
            let legacy = column
                .strip_prefix("From")
                .map(|rest| format!("FromEnd{rest}"))
                .or_else(|| column.strip_prefix("To").map(|rest| format!("ToEnd{rest}")))?;
            row.i64(&legacy)
        })
    }

    fn end_id(row: Row<'_>, column: &str) -> Option<i64> {
        Self::end_value(row, column).filter(|&id| id > 0)
    }

    fn relationships(&self) -> Vec<Relationship> {
        let mut out = Vec::new();
        for row in self.cat.relationships.iter() {
            let id = row.id();
            let from_table = Self::end_id(row, "FromTableID");
            let to_table = Self::end_id(row, "ToTableID");
            if !self.table_included(from_table) || !self.table_included(to_table) {
                continue;
            }
            let from_column = Self::end_id(row, "FromColumnID");
            let to_column = Self::end_id(row, "ToColumnID");
            let (Some(from), Some(to)) = (self.column_ref(from_column), self.column_ref(to_column))
            else {
                tracing::warn!(id, "relationship with unresolved columns skipped");
                continue;
            };
            let mut relationship = Relationship {
                name: row.string("Name").unwrap_or_else(|| id.to_string()),
                from,
                to,
                from_cardinality: codes::cardinality(
                    Self::end_value(row, "FromCardinality"),
                    Cardinality::Many,
                ),
                to_cardinality: codes::cardinality(
                    Self::end_value(row, "ToCardinality"),
                    Cardinality::One,
                ),
                cross_filtering_behavior: codes::cross_filter(row.i64("CrossFilteringBehavior")),
                is_active: row.opt_flag("IsActive").unwrap_or(true),
                security_filtering_behavior: codes::security_filter(
                    row.i64("SecurityFilteringBehavior"),
                ),
                rely_on_referential_integrity: row.flag("RelyOnReferentialIntegrity"),
                annotations: self.annotations(object_type::RELATIONSHIP, id),
                origin: self.origin("Relationship", id),
                ..Relationship::default()
            };
            if let Some(behavior) = row
                .i64("JoinOnDateBehavior")
                .filter(|&b| b == 2)
                .and_then(codes::join_on_date_behavior)
            {
                relationship.extra.insert(
                    "joinOnDateBehavior".into(),
                    PropValue::Text(behavior.into()),
                );
            }
            out.push(relationship);
        }
        out
    }

    fn expressions(&self) -> Vec<NamedExpression> {
        self.cat
            .expressions
            .iter()
            .map(|row| {
                let id = row.id();
                let name = row.string("Name").unwrap_or_default();
                let text = row.raw_str("Expression").unwrap_or_default();
                let mut expression = if row.i64("Kind") == Some(1) {
                    NamedExpression {
                        name,
                        kind: ExpressionKind::Dax,
                        expression: text.to_owned(),
                        ..NamedExpression::default()
                    }
                } else {
                    NamedExpression::m(name, text)
                };
                expression.description = row.string("Description");
                expression.query_group = self.query_group_folder(row.i64("QueryGroupID"));
                expression.lineage_tag = row.string("LineageTag");
                expression.annotations = self.annotations(object_type::EXPRESSION, id);
                expression.origin = self.origin("Expression", id);
                if let Some(tag) = row.string("SourceLineageTag") {
                    expression
                        .extra
                        .insert("sourceLineageTag".into(), PropValue::Text(tag));
                }
                if let Some(attributes) = row.string("MAttributes") {
                    expression
                        .extra
                        .insert("mAttributes".into(), PropValue::Expression(attributes));
                }
                if let Some(name) = row.string("RemoteParameterName") {
                    expression
                        .extra
                        .insert("remoteParameterName".into(), PropValue::Text(name));
                }
                if let Some(column) = self.column_ref(row.id_ref("ParameterValuesColumnID")) {
                    expression.extra.insert(
                        "parameterValuesColumn".into(),
                        PropValue::Text(format!(
                            "{}.{}",
                            tmdl_name(&column.table),
                            tmdl_name(&column.column)
                        )),
                    );
                }
                expression
            })
            .collect()
    }

    fn roles(&self) -> Vec<Role> {
        let permissions_by_role = group_by(&self.cat.table_permissions, "RoleID");
        self.cat
            .roles
            .iter()
            .map(|row| {
                let id = row.id();
                let table_permissions = permissions_by_role
                    .get(&id)
                    .into_iter()
                    .flatten()
                    .filter_map(|tp| {
                        let table = self.table_name(tp.i64("TableID"))?.to_owned();
                        let mut permission = TablePermission {
                            table,
                            filter_expression: tp.string("FilterExpression"),
                            annotations: self.annotations(object_type::TABLE_PERMISSION, tp.id()),
                            ..TablePermission::default()
                        };
                        if let Some(code) = tp.i64("MetadataPermission").filter(|&c| c != 0) {
                            let value = match code {
                                1 => "none".to_owned(),
                                2 => "read".to_owned(),
                                other => format!("metadataPermission{other}"),
                            };
                            permission
                                .extra
                                .insert("metadataPermission".into(), PropValue::Text(value));
                        }
                        Some(permission)
                    })
                    .collect();
                Role {
                    name: row.string("Name").unwrap_or_default(),
                    description: row.string("Description"),
                    model_permission: row.i64("ModelPermission").and_then(codes::model_permission),
                    table_permissions,
                    annotations: self.annotations(object_type::ROLE, id),
                    origin: self.origin("Role", id),
                    ..Role::default()
                }
            })
            .collect()
    }

    fn cultures(&self) -> Vec<Culture> {
        self.cat
            .cultures
            .iter()
            .map(|row| {
                let id = row.id();
                let linguistic = row
                    .id_ref("LinguisticMetadataID")
                    .and_then(|lm| self.cat.linguistic_metadata.by_id(lm))
                    .or_else(|| {
                        self.cat
                            .linguistic_metadata
                            .iter()
                            .find(|lm| lm.i64("CultureID") == Some(id))
                    })
                    .and_then(|lm| lm.string("Content"));
                Culture {
                    name: row.string("Name").unwrap_or_default(),
                    linguistic_metadata: linguistic,
                    annotations: self.annotations(object_type::CULTURE, id),
                    ..Culture::default()
                }
            })
            .collect()
    }
}

/// Builds the model from the loaded metadata tables.
pub(crate) fn build_model(cat: &Catalog, opts: &ModelOptions) -> Model {
    Mapper::new(cat, opts).build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_date_tables_are_detected() {
        assert_eq!(auto_date_kind("LocalDateTable_1", &[]), Some("local"));
        assert_eq!(auto_date_kind("DateTableTemplate_1", &[]), Some("template"));
        assert_eq!(
            auto_date_kind(
                "Renamed",
                &[Annotation::new("__PBI_LocalDateTable", "true")]
            ),
            Some("local")
        );
        assert_eq!(auto_date_kind("Calendar", &[]), None);
    }
}
