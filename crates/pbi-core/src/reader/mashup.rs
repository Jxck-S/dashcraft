//! Power Query documents: M scripts (`.pq`, `.m`), Power Query templates
//! (`.pqt`), Dataflow Gen2 item folders and dataflow CDM `model.json`.
//!
//! | Input | Parts read | Model |
//! |---|---|---|
//! | `.pq` / `.m` | the file: a section document (`section S; shared q = …;`) or one expression | one named expression per `shared` member (or one named after the file) |
//! | `.pqt` (OPC ZIP) | `MashupDocument.pq` (or a DataMashup-style `Formulas/Section1.m`), `MashupMetadata.json` (`queriesMetadata`, `queryGroups`), `Metadata.json` (name, description) | as `.pq`, with query groups |
//! | Dataflow Gen2 folder | `mashup.pq`, `queryMetadata.json` (`name`, `documentLocale`, `queriesMetadata`, `queryGroups`, `connections`), `.platform` | as `.pq`, with query groups |
//! | CDM `model.json` | `name`, `culture`, `entities[]` (`attributes[]`, `partitions[]`), `pbi:mashup` (`document`, `queriesMetadata`, `queryGroups`, `connectionOverrides`) | entities → tables (attributes → columns; the entity's M query → an M partition); other queries → named expressions |
//!
//! Metadata keys are matched case-insensitively (`queriesMetadata` /
//! `QueriesMetadata`); `queriesMetadata` may be an object keyed by query
//! name or an array. Query group folders are built from `parentId` chains.
//! A syntax error in the M document is an error diagnostic; the queries are
//! then split leniently at `shared` declarations and each is checked on its
//! own, so one broken query does not hide the others. Nothing is evaluated.
//! Partition locations and connection paths have secret parameters masked.

use std::collections::{BTreeSet, HashMap};

use pbi_m::{Document as MDocument, line_col, parse_document, parse_expression};
use pbi_model::{
    Column, ColumnKind, DataType, Diagnostic, Model, NamedExpression, Origin, Partition,
    PartitionMode, PartitionSource, QueryGroup, SourceFormat, Table,
};
use serde_json::Value;

use super::Builder;
use super::codes;
use super::files::{file_stem, read_json_capped, read_text_capped, root_file};
use super::outline::{
    DataflowEntity, DataflowPartition, FormatOutline, Mashup, MashupConnection, MashupQuery,
};
use crate::redact::sanitize_connection_string;

/// Largest M document read.
const M_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Largest dataflow `model.json` read.
const CDM_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Largest metadata JSON read.
const META_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Deepest query group nesting followed.
const MAX_GROUP_DEPTH: usize = 32;

/// One query of an M document.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Member {
    pub name: String,
    /// Expression text (with any `meta [...]` record).
    pub text: String,
    /// 1-based line of the declaration.
    pub line: u32,
    pub shared: bool,
    pub syntax_error: bool,
}

/// A parsed (or leniently split) M document.
#[derive(Debug, Default)]
pub(crate) struct Parsed {
    pub section: Option<String>,
    pub members: Vec<Member>,
}

fn line_of(src: &str, offset: usize) -> u32 {
    u32::try_from(line_col(src, offset).0).unwrap_or(u32::MAX)
}

/// Parses an M document. `fallback_name` names a document that is a single
/// expression. Problems become diagnostics with `origin` + line.
pub(crate) fn parse_m(
    src: &str,
    fallback_name: &str,
    origin: &str,
    diags: &mut Vec<Diagnostic>,
) -> Parsed {
    match parse_document(src) {
        Ok(MDocument::Section(section)) => Parsed {
            section: Some(section.name.text.clone()),
            members: section
                .members
                .iter()
                .map(|m| Member {
                    name: m.name.text.clone(),
                    text: m.value.span.slice(src).trim().to_owned(),
                    line: line_of(src, m.span.start),
                    shared: m.shared,
                    syntax_error: false,
                })
                .collect(),
        },
        Ok(MDocument::Expression(_)) => Parsed {
            section: None,
            members: vec![Member {
                name: fallback_name.to_owned(),
                text: src.trim().trim_end_matches(';').trim_end().to_owned(),
                line: 1,
                shared: true,
                syntax_error: false,
            }],
        },
        Err(e) => {
            diags.push(
                Diagnostic::error(codes::M_SYNTAX, format!("M syntax error: {}", e.message))
                    .with_origin(Origin::file(origin).with_line(line_of(src, e.span.start))),
            );
            let mut parsed = split_leniently(src);
            if parsed.members.is_empty() && !src.trim().is_empty() {
                parsed.members.push(Member {
                    name: fallback_name.to_owned(),
                    text: src.trim().to_owned(),
                    line: 1,
                    shared: true,
                    syntax_error: true,
                });
            }
            for m in &mut parsed.members {
                if m.syntax_error {
                    continue;
                }
                if let Err(e) = parse_expression(&m.text) {
                    m.syntax_error = true;
                    let line = m.line.saturating_add(line_of(&m.text, e.span.start) - 1);
                    diags.push(
                        Diagnostic::warning(
                            codes::M_SYNTAX,
                            format!("query `{}`: {}", m.name, e.message),
                        )
                        .with_origin(Origin::file(origin).with_line(line)),
                    );
                }
            }
            parsed
        }
    }
}

/// Reads an M identifier or `#"quoted"` name at the start of `s`.
fn read_name(s: &str) -> Option<(String, &str)> {
    if let Some(rest) = s.strip_prefix("#\"") {
        let mut name = String::new();
        let mut chars = rest.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '"' {
                if chars.peek().map(|&(_, n)| n) == Some('"') {
                    chars.next();
                    name.push('"');
                    continue;
                }
                return Some((name, rest.get(i + 1..)?));
            }
            name.push(c);
        }
        return None;
    }
    let end = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
        .unwrap_or(s.len());
    (end > 0).then(|| (s[..end].to_owned(), &s[end..]))
}

/// Splits a section document at line-leading `shared` declarations without
/// parsing the expressions (used after a syntax error).
pub(crate) fn split_leniently(src: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut starts: Vec<(usize, String, usize)> = Vec::new();
    let mut offset = 0usize;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if parsed.section.is_none()
            && let Some(rest) = trimmed.strip_prefix("section")
            && rest.starts_with(char::is_whitespace)
            && let Some((name, _)) = read_name(rest.trim_start())
        {
            parsed.section = Some(name);
        }
        let mut decl = trimmed;
        if decl.starts_with('[')
            && let Some(close) = decl.find(']')
        {
            decl = decl[close + 1..].trim_start();
        }
        if let Some(rest) = decl.strip_prefix("shared")
            && rest.starts_with(char::is_whitespace)
            && let Some((name, after)) = read_name(rest.trim_start())
            && let Some(eq) = after.trim_start().strip_prefix('=')
        {
            let expr_start = offset + indent + (trimmed.len() - eq.len());
            starts.push((offset, name, expr_start));
        }
        offset += line.len();
    }
    for (i, (decl, name, expr_start)) in starts.iter().enumerate() {
        let end = starts.get(i + 1).map_or(src.len(), |next| next.0);
        let body = src.get(*expr_start..end).unwrap_or_default();
        // The statement ends at its last `;`; attribute records or comments of
        // the next declaration may follow it.
        let body = body.rfind(';').map_or(body, |i| &body[..i]).trim();
        parsed.members.push(Member {
            name: name.clone(),
            text: body.to_owned(),
            line: line_of(src, *decl),
            shared: true,
            syntax_error: false,
        });
    }
    parsed
}

// ---------------------------------------------------------------------------
// Query metadata
// ---------------------------------------------------------------------------

/// Case-insensitive object lookup.
fn get_ci<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    let map = v.as_object()?;
    map.get(key).or_else(|| {
        map.iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    })
}

fn str_ci(v: &Value, key: &str) -> Option<String> {
    get_ci(v, key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn bool_ci(v: &Value, key: &str) -> Option<bool> {
    get_ci(v, key).and_then(Value::as_bool)
}

#[derive(Debug, Default)]
pub(crate) struct QueryMeta {
    group: Option<String>,
    load_enabled: Option<bool>,
    hidden: Option<bool>,
}

/// Query metadata by lower-cased query name, and group folders by ID.
#[derive(Debug, Default)]
pub(crate) struct Metadata {
    queries: HashMap<String, QueryMeta>,
    folders: HashMap<String, String>,
}

impl Metadata {
    /// Reads `queriesMetadata` and `queryGroups` from a metadata object.
    pub(crate) fn from_json(v: &Value) -> Self {
        let mut groups: HashMap<String, (String, Option<String>)> = HashMap::new();
        if let Some(list) = get_ci(v, "queryGroups").and_then(Value::as_array) {
            for g in list {
                if let (Some(id), Some(name)) = (str_ci(g, "id"), str_ci(g, "name")) {
                    groups.insert(id, (name, str_ci(g, "parentId")));
                }
            }
        }
        let mut folders = HashMap::new();
        for id in groups.keys() {
            let mut parts = Vec::new();
            let mut current = Some(id.clone());
            while let Some(cur) = current.take() {
                if parts.len() >= MAX_GROUP_DEPTH {
                    break;
                }
                let Some((name, parent)) = groups.get(&cur) else {
                    break;
                };
                parts.push(name.clone());
                current = parent.clone().filter(|p| p != id);
            }
            parts.reverse();
            folders.insert(id.clone(), parts.join("\\"));
        }
        let mut queries = HashMap::new();
        let mut add = |name: Option<String>, m: &Value| {
            let Some(name) = str_ci(m, "queryName").or(name) else {
                return;
            };
            let group = str_ci(m, "queryGroupId").and_then(|id| folders.get(&id).cloned());
            queries.insert(
                name.to_lowercase(),
                QueryMeta {
                    group,
                    load_enabled: bool_ci(m, "loadEnabled"),
                    hidden: bool_ci(m, "isHidden"),
                },
            );
        };
        match get_ci(v, "queriesMetadata") {
            Some(Value::Object(map)) => {
                for (name, m) in map {
                    add(Some(name.clone()), m);
                }
            }
            Some(Value::Array(list)) => {
                for m in list {
                    add(None, m);
                }
            }
            _ => {}
        }
        Self { queries, folders }
    }

    fn query(&self, name: &str) -> Option<&QueryMeta> {
        self.queries.get(&name.to_lowercase())
    }
}

/// Connections of a dataflow (`connections` / `connectionOverrides`), with
/// secrets masked.
fn connections(v: Option<&Value>) -> Vec<MashupConnection> {
    v.and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|c| MashupConnection {
                    kind: str_ci(c, "kind"),
                    path: str_ci(c, "path").map(|p| sanitize_connection_string(&p)),
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Model building
// ---------------------------------------------------------------------------

/// Adds the shared members of `parsed` (except `skip`) to `model` as named
/// expressions and returns the outline queries.
fn add_expressions(
    model: &mut Model,
    parsed: &Parsed,
    meta: &Metadata,
    origin: &str,
    entity_names: &BTreeSet<String>,
    diags: &mut Vec<Diagnostic>,
) -> Vec<MashupQuery> {
    let mut queries = Vec::new();
    // Every declared group, used or not, in folder order.
    let mut folders: BTreeSet<String> = meta
        .folders
        .values()
        .filter(|f| !f.is_empty())
        .cloned()
        .collect();
    let private: Vec<&str> = parsed
        .members
        .iter()
        .filter(|m| !m.shared)
        .map(|m| m.name.as_str())
        .collect();
    if !private.is_empty() {
        diags.push(
            Diagnostic::info(
                codes::MASHUP,
                format!(
                    "{} section-private member(s) not listed as queries: {}",
                    private.len(),
                    private.join(", ")
                ),
            )
            .with_origin(Origin::file(origin)),
        );
    }
    for m in parsed.members.iter().filter(|m| m.shared) {
        let qm = meta.query(&m.name);
        let group = qm.and_then(|q| q.group.clone()).filter(|g| !g.is_empty());
        let entity = entity_names.contains(&m.name.to_lowercase());
        queries.push(MashupQuery {
            name: m.name.clone(),
            group: group.clone(),
            load_enabled: qm.and_then(|q| q.load_enabled),
            hidden: qm.and_then(|q| q.hidden),
            entity,
            syntax_error: m.syntax_error,
        });
        if entity {
            continue;
        }
        if let Some(g) = &group {
            folders.insert(g.clone());
        }
        let mut expression = NamedExpression::m(m.name.clone(), &m.text);
        expression.query_group = group;
        expression.origin = Some(Origin::file(origin).with_line(m.line));
        model.expressions.push(expression);
    }
    let known: BTreeSet<String> = model
        .query_groups
        .iter()
        .map(|g| g.folder.clone())
        .collect();
    for folder in folders.difference(&known) {
        model.query_groups.push(QueryGroup {
            folder: folder.clone(),
            ..QueryGroup::default()
        });
    }
    queries
}

/// Reads and parses the M document at `entry`.
fn read_m(b: &mut Builder<'_>, entry: &str, fallback_name: &str) -> Option<Parsed> {
    match read_text_capped(b.container.as_ref(), entry, M_MAX_BYTES) {
        Ok((src, _)) => {
            let src = src.trim_start_matches('\u{feff}');
            Some(parse_m(src, fallback_name, entry, &mut b.doc.diagnostics))
        }
        Err(e) => {
            b.diag(
                Diagnostic::error(codes::READ_FAILED, format!("{entry}: {e}"))
                    .with_origin(Origin::file(entry)),
            );
            None
        }
    }
}

fn read_meta(b: &mut Builder<'_>, entry: &str, kind: &str) -> Option<Value> {
    b.json_part(entry, kind);
    match read_json_capped(b.container.as_ref(), entry, META_MAX_BYTES) {
        Ok((json, _)) => Some(json),
        Err(_) => None,
    }
}

fn finish(b: &mut Builder<'_>, model: Model, mashup: Mashup, what: &str) {
    let queries = mashup.queries.len();
    let errors = mashup.queries.iter().filter(|q| q.syntax_error).count();
    b.diag(Diagnostic::info(
        codes::MASHUP,
        format!(
            "{what}: {queries} quer{}{}, {} table(s); queries are not evaluated when opening",
            if queries == 1 { "y" } else { "ies" },
            if errors > 0 {
                format!(" ({errors} with syntax errors)")
            } else {
                String::new()
            },
            model.tables.len()
        ),
    ));
    b.doc.model = Some(model);
    b.outline = Some(FormatOutline::Mashup(mashup));
}

/// `.pq` / `.m` script.
pub(crate) fn read_script(b: &mut Builder<'_>, entry: &str) {
    b.doc.source.format = SourceFormat::PowerQueryScript;
    let stem = file_stem(&b.name).to_owned();
    let name = if stem.is_empty() {
        "Query1".to_owned()
    } else {
        stem
    };
    let Some(parsed) = read_m(b, entry, &name) else {
        return;
    };
    let mut model = Model {
        name: name.clone(),
        ..Model::default()
    };
    let queries = add_expressions(
        &mut model,
        &parsed,
        &Metadata::default(),
        entry,
        &BTreeSet::new(),
        &mut b.doc.diagnostics,
    );
    let mashup = Mashup {
        kind: "script".into(),
        name: Some(name),
        section: parsed.section,
        queries,
        ..Mashup::default()
    };
    finish(b, model, mashup, "M script");
}

/// `.pqt` Power Query template.
pub(crate) fn read_template(b: &mut Builder<'_>) {
    b.doc.source.format = SourceFormat::PowerQueryTemplate;
    let container = std::sync::Arc::clone(&b.container);
    let entry = root_file(container.as_ref(), "MashupDocument.pq")
        .or_else(|| {
            container
                .is_file("Formulas/Section1.m")
                .then(|| "Formulas/Section1.m".to_owned())
        })
        .or_else(|| {
            container
                .list_dir("")
                .into_iter()
                .find(|c| !c.is_dir && c.name.to_ascii_lowercase().ends_with(".pq"))
                .map(|c| c.path)
        });
    let Some(entry) = entry else {
        b.diag(Diagnostic::error(
            codes::MASHUP,
            "Power Query template without an M document (MashupDocument.pq)",
        ));
        return;
    };
    let template = root_file(container.as_ref(), "Metadata.json")
        .and_then(|m| read_meta(b, &m, "templateMetadata"))
        .unwrap_or(Value::Null);
    let meta_json = root_file(container.as_ref(), "MashupMetadata.json")
        .and_then(|m| read_meta(b, &m, "mashupMetadata"))
        .unwrap_or(Value::Null);
    let meta = Metadata::from_json(&meta_json);
    let stem = file_stem(&b.name).to_owned();
    let name = str_ci(&template, "name")
        .or_else(|| str_ci(&meta_json, "name"))
        .unwrap_or(if stem.is_empty() {
            "Template".to_owned()
        } else {
            stem
        });
    let Some(parsed) = read_m(b, &entry, &name) else {
        return;
    };
    b.consumed.insert(entry.clone());
    let mut model = Model {
        name: name.clone(),
        description: str_ci(&template, "description"),
        ..Model::default()
    };
    let queries = add_expressions(
        &mut model,
        &parsed,
        &meta,
        &entry,
        &BTreeSet::new(),
        &mut b.doc.diagnostics,
    );
    let mashup = Mashup {
        kind: "template".into(),
        name: Some(name),
        description: model.description.clone(),
        culture: str_ci(&meta_json, "documentLocale").or_else(|| str_ci(&meta_json, "locale")),
        section: parsed.section,
        queries,
        ..Mashup::default()
    };
    finish(b, model, mashup, "Power Query template");
}

/// Fabric Dataflow Gen2 item folder (`mashup.pq`, `queryMetadata.json`).
pub(crate) fn read_dataflow_folder(b: &mut Builder<'_>) {
    b.doc.source.format = SourceFormat::DataflowModel;
    let container = std::sync::Arc::clone(&b.container);
    let platform = root_file(container.as_ref(), ".platform")
        .and_then(|p| read_meta(b, &p, "platform"))
        .unwrap_or(Value::Null);
    let meta_json = root_file(container.as_ref(), "queryMetadata.json")
        .and_then(|m| read_meta(b, &m, "queryMetadata"))
        .unwrap_or(Value::Null);
    let Some(entry) = root_file(container.as_ref(), "mashup.pq") else {
        b.diag(Diagnostic::error(
            codes::MASHUP,
            "Dataflow Gen2 folder without mashup.pq",
        ));
        return;
    };
    let name = str_ci(&meta_json, "name")
        .or_else(|| get_ci(&platform, "metadata").and_then(|m| str_ci(m, "displayName")))
        .unwrap_or_else(|| "Dataflow".to_owned());
    let Some(parsed) = read_m(b, &entry, &name) else {
        return;
    };
    b.consumed.insert(entry.clone());
    let meta = Metadata::from_json(&meta_json);
    let mut model = Model {
        name: name.clone(),
        description: get_ci(&platform, "metadata").and_then(|m| str_ci(m, "description")),
        culture: str_ci(&meta_json, "documentLocale"),
        ..Model::default()
    };
    let queries = add_expressions(
        &mut model,
        &parsed,
        &meta,
        &entry,
        &BTreeSet::new(),
        &mut b.doc.diagnostics,
    );
    let mashup = Mashup {
        kind: "dataflowGen2".into(),
        name: Some(name),
        description: model.description.clone(),
        culture: model.culture.clone(),
        section: parsed.section,
        queries,
        connections: connections(get_ci(&meta_json, "connections")),
        ..Mashup::default()
    };
    finish(b, model, mashup, "Dataflow Gen2");
}

/// CDM `dataType` → model type.
fn cdm_type(t: Option<&str>) -> DataType {
    match t.map(str::to_ascii_lowercase).as_deref() {
        Some("string" | "guid" | "char") => DataType::String,
        Some("int64" | "int32" | "int16" | "integer" | "byte") => DataType::Int64,
        Some("double" | "float" | "real") => DataType::Double,
        Some("decimal") => DataType::Decimal,
        Some("boolean" | "bool") => DataType::Boolean,
        Some("datetime" | "date" | "time" | "datetimeoffset") => DataType::DateTime,
        Some("binary") => DataType::Binary,
        Some("unclassified" | "") | None => DataType::Automatic,
        Some(other) => DataType::parse(other),
    }
}

/// Dataflow CDM `model.json`.
pub(crate) fn read_cdm(b: &mut Builder<'_>, entry: &str) {
    b.doc.source.format = SourceFormat::DataflowModel;
    let root = match read_json_capped(b.container.as_ref(), entry, CDM_MAX_BYTES) {
        Ok((json, _)) => json,
        Err(e) => {
            b.diag(
                Diagnostic::error(codes::MASHUP, format!("{entry}: {e}"))
                    .with_origin(Origin::file(entry)),
            );
            return;
        }
    };
    let mashup_json = get_ci(&root, "pbi:mashup").cloned().unwrap_or(Value::Null);
    let meta = Metadata::from_json(&mashup_json);
    let name = str_ci(&root, "name").unwrap_or_else(|| file_stem(&b.name).to_owned());
    let mut model = Model {
        name: name.clone(),
        description: str_ci(&root, "description"),
        culture: str_ci(&root, "culture"),
        ..Model::default()
    };
    let parsed = match str_ci(&mashup_json, "document") {
        Some(doc) => parse_m(&doc, &name, entry, &mut b.doc.diagnostics),
        None => {
            b.diag(
                Diagnostic::warning(
                    codes::MASHUP,
                    "model.json has no pbi:mashup document: entities without M queries",
                )
                .with_origin(Origin::file(entry)),
            );
            Parsed::default()
        }
    };
    let members: HashMap<String, &Member> = parsed
        .members
        .iter()
        .filter(|m| m.shared)
        .map(|m| (m.name.to_lowercase(), m))
        .collect();

    let mut entities = Vec::new();
    let mut entity_names = BTreeSet::new();
    let list = get_ci(&root, "entities")
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    for (i, e) in list.iter().enumerate() {
        let Some(entity_name) = str_ci(e, "name") else {
            continue;
        };
        entity_names.insert(entity_name.to_lowercase());
        let entity_type = str_ci(e, "$type");
        let attributes = get_ci(e, "attributes")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let mut table = Table {
            name: entity_name.clone(),
            description: str_ci(e, "description"),
            origin: Some(Origin::file(entry).with_pointer(format!("/entities/{i}"))),
            ..Table::default()
        };
        for a in attributes {
            let Some(col) = str_ci(a, "name") else {
                continue;
            };
            table.columns.push(Column {
                description: str_ci(a, "description"),
                data_type: cdm_type(get_ci(a, "dataType").and_then(Value::as_str)),
                kind: ColumnKind::Data {
                    source_column: col.clone(),
                },
                name: col,
                ..Column::default()
            });
        }
        let reference = entity_type
            .as_deref()
            .is_some_and(|t| t.eq_ignore_ascii_case("ReferenceEntity"));
        let source = match members.get(&entity_name.to_lowercase()) {
            Some(m) if !reference => Some(PartitionSource::M {
                expression: m.text.clone(),
            }),
            _ if reference => Some(PartitionSource::Entity {
                entity_name: str_ci(e, "source").unwrap_or_else(|| entity_name.clone()),
                expression_source: str_ci(e, "modelId"),
                schema_name: None,
            }),
            _ => None,
        };
        if let Some(source) = source {
            table.partitions.push(Partition {
                name: entity_name.clone(),
                mode: PartitionMode::Import,
                source,
                ..Partition::default()
            });
        }
        let partitions = get_ci(e, "partitions")
            .and_then(Value::as_array)
            .map(|ps| {
                ps.iter()
                    .map(|p| DataflowPartition {
                        name: str_ci(p, "name").unwrap_or_default(),
                        location: str_ci(p, "location").map(|l| sanitize_connection_string(&l)),
                        refresh_time: str_ci(p, "refreshTime"),
                    })
                    .collect()
            })
            .unwrap_or_default();
        entities.push(DataflowEntity {
            name: entity_name,
            entity_type,
            attributes: table.columns.len(),
            partitions,
        });
        model.tables.push(table);
    }
    let queries = add_expressions(
        &mut model,
        &parsed,
        &meta,
        entry,
        &entity_names,
        &mut b.doc.diagnostics,
    );
    let mashup = Mashup {
        kind: "dataflowCdm".into(),
        name: Some(name),
        description: model.description.clone(),
        culture: model.culture.clone(),
        section: parsed.section,
        queries,
        entities,
        connections: connections(get_ci(&mashup_json, "connectionOverrides")),
    };
    finish(b, model, mashup, "dataflow model.json");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lenient_split_finds_shared_members() {
        let src = "section Section1;\r\n\r\nshared A = let x = 1 in x;\r\n[Description = \"d\"]\r\nshared #\"B \"\"q\"\"\" = 1 +;\r\n  shared C=2;\r\n";
        let p = split_leniently(src);
        assert_eq!(p.section.as_deref(), Some("Section1"));
        let names: Vec<(&str, &str, u32)> = p
            .members
            .iter()
            .map(|m| (m.name.as_str(), m.text.as_str(), m.line))
            .collect();
        assert_eq!(
            names,
            [
                ("A", "let x = 1 in x", 3),
                ("B \"q\"", "1 +", 5),
                ("C", "2", 6)
            ]
        );
    }

    #[test]
    fn query_groups_build_folders_and_survive_cycles() {
        let meta = Metadata::from_json(&serde_json::json!({
            "QueryGroups": [
                {"Id": "g1", "Name": "Sources"},
                {"Id": "g2", "Name": "Sales", "ParentId": "g1"},
                {"Id": "x", "Name": "Loop", "ParentId": "y"},
                {"Id": "y", "Name": "Pool", "ParentId": "x"}
            ],
            "QueriesMetadata": {
                "Orders": {"QueryGroupId": "g2", "LoadEnabled": false, "IsHidden": true},
                "Other": {"queryName": "Renamed"}
            }
        }));
        let orders = meta.query("orders").unwrap();
        assert_eq!(orders.group.as_deref(), Some("Sources\\Sales"));
        assert_eq!(orders.load_enabled, Some(false));
        assert_eq!(orders.hidden, Some(true));
        assert!(meta.query("renamed").is_some());
        assert!(meta.folders.get("x").is_some_and(|f| f.len() < 1000));
    }

    #[test]
    fn cdm_types_map() {
        assert_eq!(cdm_type(Some("int64")), DataType::Int64);
        assert_eq!(cdm_type(Some("dateTimeOffset")), DataType::DateTime);
        assert_eq!(cdm_type(None), DataType::Automatic);
    }
}
