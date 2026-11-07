//! The names a DAX expression can refer to.

use std::collections::{HashMap, HashSet};

use pbi_model::{ColumnKind, Model, PropValue};

/// Tables with their columns and measures, and user-defined functions, as
/// the language service sees them. DAX names are case-insensitive: every
/// lookup ignores case.
#[derive(Debug, Clone, Default)]
pub struct Schema {
    tables: Vec<SchemaTable>,
    table_keys: HashMap<String, usize>,
    /// Lower-cased measure name → home table index.
    measure_keys: HashMap<String, usize>,
    /// Lower-cased names of every column of every table.
    column_keys: HashSet<String>,
    functions: Vec<String>,
    function_keys: HashSet<String>,
    /// The model is not known: references are not checked.
    unknown: bool,
}

/// What hover shows about a table, column or measure (from the model).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameDetails {
    /// Column data type in TMDL spelling (`string`, `int64`, `dateTime`, …).
    pub data_type: Option<String>,
    /// DAX of a measure or calculated column.
    pub expression: Option<String>,
    pub format_string: Option<String>,
    pub description: Option<String>,
    pub hidden: bool,
}

/// One table of a [`Schema`].
#[derive(Debug, Clone, Default)]
pub struct SchemaTable {
    name: String,
    columns: Vec<String>,
    measures: Vec<String>,
    column_keys: HashSet<String>,
    measure_keys: HashSet<String>,
    open: bool,
    details: NameDetails,
    column_details: HashMap<String, NameDetails>,
    measure_details: HashMap<String, NameDetails>,
}

impl SchemaTable {
    /// The table name as written in the model.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Column names in model order.
    #[must_use]
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// Measure names in model order.
    #[must_use]
    pub fn measures(&self) -> &[String] {
        &self.measures
    }

    /// The columns are not known (a query's `DEFINE TABLE`): any column
    /// reference on it is accepted.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Whether `name` is a column of this table.
    #[must_use]
    pub fn has_column(&self, name: &str) -> bool {
        self.column_keys.contains(&name.to_lowercase())
    }

    /// Whether `name` is a measure of this table.
    #[must_use]
    pub fn has_measure(&self, name: &str) -> bool {
        self.measure_keys.contains(&name.to_lowercase())
    }

    /// The table's description and hidden flag.
    #[must_use]
    pub fn details(&self) -> &NameDetails {
        &self.details
    }

    /// Model details of the column `name`, if the schema has them.
    #[must_use]
    pub fn column_details(&self, name: &str) -> Option<&NameDetails> {
        self.column_details.get(&name.to_lowercase())
    }

    /// Model details of the measure `name`, if the schema has them.
    #[must_use]
    pub fn measure_details(&self, name: &str) -> Option<&NameDetails> {
        self.measure_details.get(&name.to_lowercase())
    }

    /// The name as written in the schema of a column or measure of this
    /// table (any case).
    #[must_use]
    pub fn spelling(&self, name: &str) -> Option<&str> {
        let key = name.to_lowercase();
        self.columns
            .iter()
            .chain(&self.measures)
            .find(|n| n.to_lowercase() == key)
            .map(String::as_str)
    }
}

impl Schema {
    /// An empty schema: every table, column and measure reference is
    /// unknown.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A schema for text without a model: table, column and measure
    /// references are not checked, only syntax and functions.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            unknown: true,
            ..Self::default()
        }
    }

    /// Whether this is [`Schema::unknown`].
    #[must_use]
    pub fn is_unknown(&self) -> bool {
        self.unknown
    }

    /// The tables, columns, measures and user-defined functions of `model`.
    #[must_use]
    pub fn from_model(model: &Model) -> Self {
        let mut schema = Self::new();
        for table in &model.tables {
            let index = schema.add_table(&table.name);
            for column in &table.columns {
                schema.add_column_at(index, &column.name);
            }
            for measure in &table.measures {
                schema.add_measure_at(index, &measure.name);
            }
            let Some(t) = schema.tables.get_mut(index) else {
                continue;
            };
            t.details = NameDetails {
                description: table.description.clone(),
                hidden: table.is_hidden,
                ..NameDetails::default()
            };
            for column in &table.columns {
                let expression = match &column.kind {
                    ColumnKind::Calculated { expression } => Some(expression.clone()),
                    _ => None,
                };
                t.column_details.insert(
                    column.name.to_lowercase(),
                    NameDetails {
                        data_type: Some(column.data_type.as_str().to_string()),
                        expression,
                        format_string: column.format_string.clone(),
                        description: column.description.clone(),
                        hidden: column.is_hidden,
                    },
                );
            }
            for measure in &table.measures {
                t.measure_details.insert(
                    measure.name.to_lowercase(),
                    NameDetails {
                        data_type: None,
                        expression: Some(measure.expression.clone()),
                        format_string: measure.format_string.clone(),
                        description: measure.description.clone(),
                        hidden: measure.is_hidden,
                    },
                );
            }
        }
        for (key, value) in &model.extra {
            if key.eq_ignore_ascii_case("function") || key.eq_ignore_ascii_case("functions") {
                for name in function_names(value) {
                    schema.add_function(&name);
                }
            }
        }
        schema
    }

    /// Tables in model order.
    #[must_use]
    pub fn tables(&self) -> &[SchemaTable] {
        &self.tables
    }

    /// Table by name.
    #[must_use]
    pub fn table(&self, name: &str) -> Option<&SchemaTable> {
        self.table_keys
            .get(&name.to_lowercase())
            .and_then(|&i| self.tables.get(i))
    }

    /// The home table of the measure `name`.
    #[must_use]
    pub fn measure_table(&self, name: &str) -> Option<&SchemaTable> {
        self.measure_keys
            .get(&name.to_lowercase())
            .and_then(|&i| self.tables.get(i))
    }

    /// Whether some table has a column `name`.
    #[must_use]
    pub fn has_column_anywhere(&self, name: &str) -> bool {
        self.column_keys.contains(&name.to_lowercase())
    }

    /// User-defined function names.
    #[must_use]
    pub fn functions(&self) -> &[String] {
        &self.functions
    }

    /// Whether `name` is a user-defined function.
    #[must_use]
    pub fn has_function(&self, name: &str) -> bool {
        self.function_keys.contains(&name.to_lowercase())
    }

    /// Adds a table (or returns the existing one's index).
    pub fn add_table(&mut self, name: &str) -> usize {
        let key = name.to_lowercase();
        if let Some(&i) = self.table_keys.get(&key) {
            return i;
        }
        let i = self.tables.len();
        self.tables.push(SchemaTable {
            name: name.to_string(),
            ..SchemaTable::default()
        });
        self.table_keys.insert(key, i);
        i
    }

    /// Adds a table whose columns are unknown (any column is accepted).
    pub fn add_open_table(&mut self, name: &str) -> usize {
        let i = self.add_table(name);
        if let Some(t) = self.tables.get_mut(i) {
            t.open = true;
        }
        i
    }

    /// Adds the column `table[name]`, adding the table if needed.
    pub fn add_column(&mut self, table: &str, name: &str) {
        let i = self.add_table(table);
        self.add_column_at(i, name);
    }

    /// Adds the measure `table[name]`, adding the table if needed.
    pub fn add_measure(&mut self, table: &str, name: &str) {
        let i = self.add_table(table);
        self.add_measure_at(i, name);
    }

    /// Adds (or redefines) the measure `table[name]` with its details (a
    /// report-level measure, a query `MEASURE`).
    pub fn add_measure_with(&mut self, table: &str, name: &str, details: NameDetails) {
        let i = self.add_table(table);
        self.add_measure_at(i, name);
        if let Some(t) = self.tables.get_mut(i) {
            t.measure_details.insert(name.to_lowercase(), details);
        }
    }

    /// Adds (or redefines) the column `table[name]` with its details (a
    /// query `COLUMN`).
    pub fn add_column_with(&mut self, table: &str, name: &str, details: NameDetails) {
        let i = self.add_table(table);
        self.add_column_at(i, name);
        if let Some(t) = self.tables.get_mut(i) {
            t.column_details.insert(name.to_lowercase(), details);
        }
    }

    /// Adds a user-defined function.
    pub fn add_function(&mut self, name: &str) {
        if self.function_keys.insert(name.to_lowercase()) {
            self.functions.push(name.to_string());
        }
    }

    fn add_column_at(&mut self, index: usize, name: &str) {
        let key = name.to_lowercase();
        if let Some(t) = self.tables.get_mut(index)
            && t.column_keys.insert(key.clone())
        {
            t.columns.push(name.to_string());
            self.column_keys.insert(key);
        }
    }

    fn add_measure_at(&mut self, index: usize, name: &str) {
        let key = name.to_lowercase();
        if let Some(t) = self.tables.get_mut(index)
            && t.measure_keys.insert(key.clone())
        {
            t.measures.push(name.to_string());
            self.measure_keys.entry(key).or_insert(index);
        }
    }
}

/// Names of the user-defined functions in a model `function` / `functions`
/// property (TMDL objects or TMSL JSON).
fn function_names(value: &PropValue) -> Vec<String> {
    match value {
        PropValue::Object(raw) => raw.name.iter().cloned().collect(),
        PropValue::List(items) => items.iter().flat_map(function_names).collect(),
        PropValue::Json(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|f| f.get("name").and_then(serde_json::Value::as_str))
            .map(str::to_string)
            .collect(),
        PropValue::Json(serde_json::Value::Object(f)) => f
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}
