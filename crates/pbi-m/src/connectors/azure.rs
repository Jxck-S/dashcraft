//! `AzureStorage.Blobs(url, optional options)`: the navigation table of a container.

use std::sync::Arc;

use super::BlobResolver;
use crate::ast::PrimitiveType;
use crate::eval::interp::Interp;
use crate::merror::MError;
use crate::stdlib::args::text_arg;
use crate::value::{BinarySource, BinaryValue, Lazy, RecordValue, TableValue, TypeValue, Value};

/// The lazily loaded `Content` of a blob.
#[derive(Debug)]
struct BlobContent {
    resolver: Arc<dyn BlobResolver>,
    container: String,
    name: String,
    size: Option<u64>,
}

impl BinarySource for BlobContent {
    fn load(&self, max_bytes: u64) -> Result<Vec<u8>, MError> {
        self.resolver
            .open_limited(&self.container, &self.name, max_bytes)
    }

    fn size_hint(&self) -> Option<u64> {
        self.size
    }
}

fn content_type(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        ".csv" => "text/csv",
        ".txt" => "text/plain",
        ".json" => "application/json",
        ".xml" => "application/xml",
        ".parquet" => "application/octet-stream",
        ".xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        _ => "application/octet-stream",
    }
}

pub(crate) fn azure_storage_blobs(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let url = text_arg(args, 0, "AzureStorage.Blobs")?
        .ok_or_else(|| MError::expression("AzureStorage.Blobs requires a container URL."))?;
    let url = url.trim().trim_end_matches('/').to_owned();
    let entries = it.opts.resolver.list(&url)?;
    it.check_rows(entries.len())?;
    let names: Vec<Arc<str>> = [
        "Content",
        "Name",
        "Extension",
        "Date accessed",
        "Date modified",
        "Date created",
        "Attributes",
        "Folder Path",
    ]
    .into_iter()
    .map(Arc::from)
    .collect();
    let dtz = TypeValue::primitive(PrimitiveType::DateTimeZone).nullable();
    let types = vec![
        TypeValue::primitive(PrimitiveType::Binary),
        TypeValue::text(),
        TypeValue::text(),
        dtz.clone(),
        dtz.clone(),
        dtz,
        TypeValue::primitive(PrimitiveType::Record).nullable(),
        TypeValue::text(),
    ];
    let mut rows = Vec::with_capacity(entries.len());
    for e in entries {
        it.tick()?;
        let file = e.name.rsplit('/').next().unwrap_or(&e.name);
        let ext = file.rfind('.').map_or("", |i| &file[i..]).to_owned();
        let folder = match e.name.rfind('/') {
            Some(i) => format!("{url}/{}", &e.name[..=i]),
            None => format!("{url}/"),
        };
        let attributes = RecordValue::from_pairs([
            ("Content Type", Value::text(content_type(&ext))),
            (
                "Kind",
                Value::text(if ext.eq_ignore_ascii_case(".csv") {
                    "Text"
                } else {
                    "File"
                }),
            ),
            (
                "Size",
                e.size.map_or(Value::Null, |s| Value::Number(s as f64)),
            ),
        ])?;
        let modified = e
            .modified
            .map_or(Value::Null, |m| Value::DateTimeZone(m.fixed_offset()));
        let content = BinaryValue::lazy(Arc::new(BlobContent {
            resolver: it.opts.resolver.clone(),
            container: url.clone(),
            name: e.name.clone(),
            size: e.size,
        }));
        rows.push(
            [
                Value::Binary(content),
                Value::text(e.name.as_str()),
                Value::text(ext),
                Value::Null,
                modified,
                Value::Null,
                Value::Record(attributes),
                Value::text(folder),
            ]
            .into_iter()
            .map(Lazy::Value)
            .collect(),
        );
    }
    let table = TableValue::from_rows(names, rows)?;
    Ok(Value::Table(table.with_types(types)))
}
