//! The M standard library subset.
//!
//! Library functions are native [`FunctionValue`]s registered by name (`Table.SelectRows`);
//! enumeration constants (`QuoteStyle.Csv`) are numbers and type constants (`Int64.Type`) are
//! type values, as in Power Query. [`library_names`] lists everything that is available.
//! Unknown dotted names evaluate to [`MError::Unsupported`]; data-access functions get a hint
//! explaining that DashCraft never reads files, databases or the network from M.

pub(crate) mod args;
pub(crate) mod convert;
mod csv;
mod datetime;
mod list;
mod number;
mod record;
pub(crate) mod table;
mod text;
mod value_fns;

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use crate::merror::{DATA_ACCESS_HINT, LOAD_MANUALLY_HINT, MError};
use crate::value::{FunctionValue, NativeFn, NativeImpl, Value};

pub(crate) struct Library {
    map: HashMap<&'static str, Value>,
    order: Vec<&'static str>,
}

impl Library {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.map.get(name)
    }

    pub fn entries(&self) -> impl Iterator<Item = (&'static str, &Value)> {
        self.order
            .iter()
            .filter_map(|n| self.map.get(n).map(|v| (*n, v)))
    }
}

#[derive(Default)]
pub(crate) struct Builder {
    map: HashMap<&'static str, Value>,
    order: Vec<&'static str>,
}

fn param_specs(params: &[&'static str]) -> Vec<(&'static str, bool)> {
    params
        .iter()
        .map(|p| match p.strip_prefix('?') {
            Some(name) => (name, true),
            None => (*p, false),
        })
        .collect()
}

impl Builder {
    /// Registers a native function; parameter names prefixed with `?` are optional.
    pub fn func(&mut self, name: &'static str, params: &[&'static str], f: NativeFn) {
        let v = Value::Function(FunctionValue::native(
            name,
            &param_specs(params),
            NativeImpl::Static(f),
        ));
        self.value(name, v);
    }

    /// Registers a native function that receives arguments with their metadata.
    pub fn raw_func(&mut self, name: &'static str, params: &[&'static str], f: NativeFn) {
        let v = Value::Function(FunctionValue::native_raw(name, &param_specs(params), f));
        self.value(name, v);
    }

    pub fn value(&mut self, name: &'static str, v: Value) {
        if self.map.insert(name, v).is_none() {
            self.order.push(name);
        }
    }

    pub fn consts(&mut self, items: &[(&'static str, f64)]) {
        for (name, n) in items {
            self.value(name, Value::Number(*n));
        }
    }
}

pub(crate) fn library() -> &'static Library {
    static LIB: OnceLock<Library> = OnceLock::new();
    LIB.get_or_init(|| {
        let mut b = Builder::default();
        table::register(&mut b);
        text::register(&mut b);
        list::register(&mut b);
        record::register(&mut b);
        number::register(&mut b);
        datetime::register(&mut b);
        csv::register(&mut b);
        value_fns::register(&mut b);
        b.func(
            "AzureStorage.Blobs",
            &["account", "?options"],
            crate::connectors::azure_storage_blobs,
        );
        Library {
            map: b.map,
            order: b.order,
        }
    })
}

/// Names of all library functions and constants DashCraft implements, in registration order.
pub fn library_names() -> Vec<&'static str> {
    library()
        .entries()
        .map(|(n, _)| n)
        .filter(|n| !n.starts_with('#'))
        .collect()
}

/// Prefixes of data-access / external functions that DashCraft deliberately does not evaluate.
const DATA_ACCESS_PREFIXES: &[&str] = &[
    "File.",
    "Folder.",
    "Web.",
    "Sql.",
    "Odbc.",
    "OleDb.",
    "OData.",
    "SharePoint.",
    "Excel.",
    "Access.",
    "AnalysisServices.",
    "AzureStorage.",
    "AzureDataExplorer.",
    "Kusto.",
    "Oracle.",
    "MySQL.",
    "PostgreSQL.",
    "Snowflake.",
    "Databricks.",
    "GoogleBigQuery.",
    "Salesforce.",
    "Exchange.",
    "ActiveDirectory.",
    "PowerBI.",
    "PowerPlatform.",
    "Dataflows.",
    "Hdfs.",
    "HdInsight.",
    "Spark.",
    "Hive.",
    "Teradata.",
    "Sybase.",
    "DB2.",
    "Informix.",
    "SapHana.",
    "SapBusinessWarehouse.",
    "Cdm.",
    "Pdf.",
    "Xml.",
    "AzureEnterprise.",
    "CommonDataService.",
    "Dynamics.",
    "Extension.",
    "Cube.",
];

pub(crate) fn unknown_name(name: &str) -> MError {
    if name.starts_with("Python.") || name.starts_with("R.") {
        return MError::unsupported(
            name,
            "DashCraft never executes Python or R scripts. Use \"Load CSV manually…\" to provide this table's data.",
        );
    }
    if DATA_ACCESS_PREFIXES.iter().any(|p| name.starts_with(p)) {
        return MError::unsupported(name, DATA_ACCESS_HINT);
    }
    if name.contains('.') && name.starts_with(|c: char| c.is_ascii_uppercase()) {
        return MError::unsupported(name, LOAD_MANUALLY_HINT);
    }
    MError::Evaluation {
        reason: "Expression.Error".into(),
        message: format!("The name '{name}' wasn't recognized. Make sure it's spelled correctly."),
        detail: None,
        location: None,
    }
}

pub(crate) fn missing_column(name: &str) -> MError {
    MError::Evaluation {
        reason: "Expression.Error".into(),
        message: format!("The column '{name}' of the table wasn't found."),
        detail: Some(Box::new(Value::Text(Arc::from(name)))),
        location: None,
    }
}

/// `MissingField.Error` / `.Ignore` / `.UseNull`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MissingField {
    Error,
    Ignore,
    UseNull,
}

impl MissingField {
    pub fn from_arg(v: &Value) -> Result<Self, MError> {
        match v.plain() {
            Value::Null => Ok(MissingField::Error),
            Value::Number(n) if *n == 0.0 => Ok(MissingField::Error),
            Value::Number(n) if *n == 1.0 => Ok(MissingField::Ignore),
            Value::Number(n) if *n == 2.0 => Ok(MissingField::UseNull),
            other => Err(MError::expression(format!(
                "{} is not a valid MissingField value.",
                crate::value::format::describe(other)
            ))),
        }
    }
}
