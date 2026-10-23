//! The database definition (`<database>.db.xml`) stored next to the model folder: an
//! XML `Load/ObjectDefinition/Database` document with the name and compatibility level.
//! Only a few scalar children of `Database` are read (it also holds an encrypted
//! impersonation password, which is ignored).

use pbi_model::{Database, PropValue};

use crate::abf::{AbfArchive, xml};

const WHAT: &str = "database definition";

/// Reads the database definition of `archive`, if present and parseable.
pub(crate) fn database_from_archive(archive: &AbfArchive) -> Option<Database> {
    let file = archive.files().iter().find(|f| {
        !f.logical_path.contains('/') && f.logical_path.to_lowercase().ends_with(".db.xml")
    })?;
    let bytes = archive.read_file(file).ok()?;
    match parse_database_xml(&bytes) {
        Ok(db) => db,
        Err(err) => {
            tracing::warn!(%err, "database definition not readable");
            None
        }
    }
}

pub(crate) fn parse_database_xml(bytes: &[u8]) -> Result<Option<Database>, crate::abf::AbfError> {
    let text = xml::decode_document(bytes, WHAT)?;
    let doc = xml::parse(&text, WHAT)?;
    let Some(node) = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "Database")
    else {
        return Ok(None);
    };
    let text_of = |name: &str| {
        node.children()
            .find(|c| c.is_element() && c.tag_name().name() == name)
            .and_then(|c| c.text())
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
    };
    let mut database = Database {
        name: text_of("Name"),
        compatibility_level: text_of("CompatibilityLevel").and_then(|v| v.parse().ok()),
        description: text_of("Description"),
        ..Database::default()
    };
    if let Some(id) = text_of("ID") {
        database.extra.insert("id".into(), PropValue::Text(id));
    }
    if let Some(language) = text_of("Language").and_then(|v| v.parse::<i64>().ok()) {
        database.extra.insert(
            "language".into(),
            PropValue::Json(serde_json::Value::from(language)),
        );
    }
    if let Some(engine) = text_of("StorageEngineUsed") {
        database
            .extra
            .insert("storageEngineUsed".into(), PropValue::Text(engine));
    }
    Ok(Some(database))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_name_and_compatibility_level() {
        let xml = r#"<Load xmlns="http://schemas.microsoft.com/analysisservices/2003/engine" xmlns:ddl200="http://schemas.microsoft.com/analysisservices/2010/engine/200"><ObjectDefinition><Database><Name>db1</Name><ID>db1</ID><Language>1033</Language><ddl200:CompatibilityLevel>1606</ddl200:CompatibilityLevel><DataSourceImpersonationInfo><Password>x</Password></DataSourceImpersonationInfo></Database></ObjectDefinition></Load>"#;
        let db = parse_database_xml(xml.as_bytes()).unwrap().unwrap();
        assert_eq!(db.name.as_deref(), Some("db1"));
        assert_eq!(db.compatibility_level, Some(1606));
        assert_eq!(
            db.extra.get("language").and_then(|v| v.to_json().as_i64()),
            Some(1033)
        );
        assert!(!format!("{db:?}").contains("Password"));
        assert_eq!(parse_database_xml(b"<Load/>").unwrap(), None);
        assert!(parse_database_xml(b"<Load>").is_err());
    }
}
