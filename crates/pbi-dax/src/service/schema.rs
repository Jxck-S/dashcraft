//! The names a DAX expression can refer to.

use std::collections::{HashMap, HashSet};

use pbi_model::{Model, PropValue};

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

/// One table of a [`Schema`].
#[derive(Debug, Clone, Default)]
pub struct SchemaTable {
    name: String,
    columns: Vec<String>,
    measures: Vec<String>,
    column_keys: HashSet<String>,
    measure_keys: HashSet<String>,
    open: bool,
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
