//! Legacy XML metadata (compatibility level 1100/1103) → [`pbi_model::Model`] and the
//! [`StorageMap`] of its VertiPaq data.
//!
//! Backups written before tabular models moved to `metadata.sqlitedb` — Power Pivot
//! workbooks (`xl/model/item.data`) and SSAS 2012/2014 tabular backups — describe the
//! model with ASSL `Load` documents, one per object:
//!
//! | File (logical path) | Object | Used for |
//! |---|---|---|
//! | `<db>.db.xml` | `Database` | name, compatibility level ([`super::database`]) |
//! | `Model.<n>.cub.xml` | `Cube` | model name; `Dimensions/Dimension` = tables in order (`Name`, `DimensionID`, `Visible`) |
//! | `<dim id>.<n>.dim.xml` | `Dimension` | table description; `Attributes/Attribute` = columns; `Hierarchies` = user hierarchies; `Relationships/Relationship` = relationships |
//! | `Model.<n>.cub/MdxScript.<n>.scr.xml` | `MdxScript` | measures: `CREATE MEASURE 'T'[M]=<DAX>;` commands (annotations `Table`, `FullName`) |
//! | `<id>.<n>.prt.xml` (under `Model.<n>.cub/<mg id>.<n>.det/`) | `Partition` | `QueryBinding/QueryDefinition` → query partitions of the table whose dimension ID is the `ParentObject/MeasureGroupID` |
//! | `<id>.<n>.ds.xml` | `DataSource` | data source names (connection strings are never read) |
//!
//! Columns: attribute `Type` `RowNumber` is the internal row-number column (skipped
//! unless [`ModelOptions::include_row_number_columns`]); `KeyColumns/KeyColumn/Source`
//! is a `ColumnBinding` (data column; `ColumnID` is the XML-encoded source column, e.g.
//! `Plant_x0020_ID`) or an `ExpressionBinding` (DAX calculated column); the key
//! `DataType` (`WChar`, `BigInt`, `Double`, `Currency`, `Date`, `Boolean`, ...) gives
//! the column type (`Empty` for calculated columns → automatic);
//! `AttributeHierarchyVisible=false` hides the column.
//!
//! Storage: each table's `<dim id>.<n>.tbl.xml` (an `XMSimpleTable` object tree) lists
//! its columns (`XMRawColumn`, named by attribute ID) with `ColumnStats`, the segments
//! (`XMColumnSegment`: `Records`, `CompressionInfo` class such as
//! `XMHybridRLECompressionInfo<class XMRENoSplitCompressionInfo<6>>` with its
//! `RLECompression` facet, `SubSegment`, `ColumnSegmentStats`) and the data objects
//! (`XMHashDataDictionary<XM_String>` named after its `.dictionary` file, or
//! `XMValueDataDictionary<XM_Long>` with `BaseId`/`Magnitude`, and one
//! `XMRawColumnPartitionDataObject` per partition named after its `.idf`). There are no
//! `.idfmeta` files: the segment metadata goes inline into the [`StorageMap`]
//! ([`ColumnPartitionFiles::meta`](super::ColumnPartitionFiles::meta)), so the regular
//! VertiPaq decoder reads the data. Calculated columns (key type `Empty`) take their
//! type from the dictionary class (`XM_String`/`XM_Long`/`XM_Real`). Tables whose
//! storage cannot be read keep their schema and get a
//! [`codes::XML_METADATA_PART`](crate::vertipaq::codes::XML_METADATA_PART) warning.
//!
//! Files are matched by suffix, not folder: each backup-log file group has its own root,
//! so logical paths of nested groups (cube, measure groups, partitions) are relative to
//! different folders. Ownership comes from each document's `ParentObject`.
//!
//! References: ASSL element names as found in Power Pivot backups, cross-checked with
//! pbixray `meta/xml_source.py` and `xldm/*.py` (MIT); see `REFERENCES.md`.

use std::collections::HashMap;

use pbi_model::{
    Annotation, Cardinality, Column, ColumnKind, ColumnRef, DataType, Diagnostic, Hierarchy, Level,
    Measure, Model, Origin, Partition, PartitionMode, PartitionSource, PropValue, Relationship,
    Table,
};
use roxmltree::Node;

use super::codes::ColumnKindCode;
use super::{
    ColumnPartitionFiles, ColumnStatistics, ColumnStorage, DictionaryEncoding, DictionaryStorage,
    LoadedModel, MetadataError, MetadataFormat, ModelOptions, StorageMap, TableStorage,
};
use crate::abf::{AbfArchive, AbfError, AbfFile, xml};
use crate::vertipaq::codes;
use crate::vertipaq::idfmeta::{
    ColumnPartitionMeta, GENERAL_CLASS, HYBRID_RLE_CLASS, NO_SPLIT_CLASS_BASE, RleFacet,
    SegmentMeta, SegmentStats, SubSegmentMeta, XM123_CLASS,
};

/// Largest XML metadata document parsed (the biggest real ones are well below 1 MiB).
const MAX_XML_BYTES: usize = 64 << 20;

/// `true` if the archive describes its model with legacy XML metadata (a top-level
/// `*.cub.xml`).
pub(crate) fn has_xml_metadata(archive: &AbfArchive) -> bool {
    cube_file(archive).is_some()
}

fn ends_with_ci(s: &str, suffix: &str) -> bool {
    s.len() >= suffix.len()
        && s.is_char_boundary(s.len() - suffix.len())
        && s[s.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

fn files_of_kind<'a>(archive: &'a AbfArchive, kind: &str) -> Vec<&'a AbfFile> {
    let suffix = format!(".{kind}.xml");
    archive
        .files()
        .iter()
        .filter(|f| ends_with_ci(&f.logical_path, &suffix))
        .collect()
}

fn cube_file(archive: &AbfArchive) -> Option<&AbfFile> {
    let cubes = files_of_kind(archive, "cub");
    cubes
        .iter()
        .find(|f| f.file_name().starts_with("Model."))
        .or_else(|| cubes.first())
        .copied()
}

/// `<id>.<n>.<kind>.xml` → `<id>` (the object ID; IDs may contain dots).
fn object_id(file_name: &str, kind: &str) -> Option<String> {
    let suffix = format!(".{kind}.xml");
    if !ends_with_ci(file_name, &suffix) {
        return None;
    }
    let stem = &file_name[..file_name.len() - suffix.len()];
    let (id, version) = stem.rsplit_once('.')?;
    version
        .chars()
        .all(|c| c.is_ascii_digit())
        .then(|| id.to_owned())
}

/// `<id>.<n>.det` folder name → `<id>`.
fn folder_object_id(folder: &str, kind: &str) -> Option<String> {
    let stem = folder.strip_suffix(&format!(".{kind}"))?;
    let (id, version) = stem.rsplit_once('.')?;
    version
        .chars()
        .all(|c| c.is_ascii_digit())
        .then(|| id.to_owned())
}

/// Decodes XML-encoded names (`Plant_x0020_ID` → `Plant ID`).
pub(crate) fn decode_xml_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut rest = name;
    while let Some(i) = rest.find("_x") {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let decoded = tail
            .get(2..6)
            .filter(|hex| hex.chars().all(|c| c.is_ascii_hexdigit()))
            .filter(|_| tail.get(6..7) == Some("_"))
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .and_then(char::from_u32);
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[7..];
            }
            None => {
                out.push_str("_x");
                rest = &tail[2..];
            }
        }
    }
    out.push_str(rest);
    out
}

// ---------------------------------------------------------------------------
// XML helpers (local names, namespaces ignored)
// ---------------------------------------------------------------------------

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

fn children<'a, 'i: 'a>(node: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> {
    node.children()
        .filter(move |n| n.is_element() && n.tag_name().name() == name)
}

fn text(node: Node<'_, '_>, name: &str) -> Option<String> {
    let element = child(node, name)?;
    let value: String = element
        .children()
        .filter(Node::is_text)
        .filter_map(|n| n.text())
        .collect();
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn flag(node: Node<'_, '_>, name: &str) -> Option<bool> {
    text(node, name).map(|v| v.eq_ignore_ascii_case("true") || v == "1")
}

/// `xsi:type` of a node without its prefix (`ddl200_200:ExpressionBinding` →
/// `ExpressionBinding`).
fn xsi_type(node: Node<'_, '_>) -> Option<String> {
    node.attributes()
        .find(|a| a.name() == "type")
        .map(|a| a.value().rsplit(':').next().unwrap_or_default().to_owned())
}

/// The object element of a `Load/ObjectDefinition/<name>` document (or the root itself).
fn object<'a, 'i>(doc: &'a roxmltree::Document<'i>, name: &str) -> Option<Node<'a, 'i>> {
    doc.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

fn read_text(archive: &AbfArchive, file: &AbfFile, what: &'static str) -> Result<String, AbfError> {
    let bytes = archive.read_file(file)?;
    if bytes.len() > MAX_XML_BYTES {
        return Err(AbfError::LimitExceeded {
            what,
            value: bytes.len() as u64,
            limit: MAX_XML_BYTES as u64,
        });
    }
    xml::decode_document(&bytes, what)
}

// ---------------------------------------------------------------------------
// Parsed objects
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Attribute {
    id: String,
    name: String,
    description: Option<String>,
    row_number: bool,
    data_type: Option<String>,
    source_column: Option<String>,
    expression: Option<String>,
    visible: bool,
    display_folder: Option<String>,
    format_string: Option<String>,
    order_by_attribute: Option<String>,
}

#[derive(Debug, Default)]
struct RelationshipEnd {
    dimension: String,
    attribute: String,
    multiplicity: Option<String>,
}

#[derive(Debug, Default)]
struct XmlRelationship {
    id: String,
    visible: bool,
    from: RelationshipEnd,
    to: RelationshipEnd,
}

#[derive(Debug, Default)]
struct XmlHierarchy {
    name: String,
    description: Option<String>,
    visible: bool,
    display_folder: Option<String>,
    levels: Vec<(String, String)>,
}

#[derive(Debug, Default)]
struct Dimension {
    id: String,
    name: String,
    description: Option<String>,
    attributes: Vec<Attribute>,
    hierarchies: Vec<XmlHierarchy>,
    relationships: Vec<XmlRelationship>,
    annotations: Vec<Annotation>,
    path: String,
}

#[derive(Debug)]
struct CubeDimension {
    id: String,
    name: String,
    visible: bool,
}

#[derive(Debug, Default)]
struct XmlPartition {
    /// `ParentObject/MeasureGroupID`.
    group: Option<String>,
    name: String,
    query: Option<String>,
    data_source: Option<String>,
    path: String,
}

fn parse_annotations(node: Node<'_, '_>) -> Vec<Annotation> {
    child(node, "Annotations")
        .map(|a| {
            children(a, "Annotation")
                .filter_map(|n| {
                    Some(Annotation::new(
                        text(n, "Name")?,
                        text(n, "Value").unwrap_or_default(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_attribute(node: Node<'_, '_>) -> Attribute {
    let key = child(node, "KeyColumns").and_then(|k| child(k, "KeyColumn"));
    let source = key.and_then(|k| child(k, "Source"));
    let binding = source.and_then(xsi_type).unwrap_or_default();
    let mut attribute = Attribute {
        id: text(node, "ID").unwrap_or_default(),
        name: text(node, "Name").unwrap_or_default(),
        description: text(node, "Description"),
        row_number: text(node, "Type").is_some_and(|t| t == "RowNumber")
            || binding == "RowNumberBinding",
        data_type: key.and_then(|k| text(k, "DataType")),
        visible: flag(node, "AttributeHierarchyVisible").unwrap_or(true),
        display_folder: text(node, "AttributeHierarchyDisplayFolder"),
        format_string: text(node, "FormatString"),
        ..Attribute::default()
    };
    if attribute.name.is_empty() {
        attribute.name = attribute.id.clone();
    }
    if text(node, "OrderBy").as_deref() == Some("AttributeKey") {
        attribute.order_by_attribute = text(node, "OrderByAttributeID");
    }
    match (binding.as_str(), source) {
        ("ExpressionBinding", Some(s)) => attribute.expression = text(s, "Expression"),
        ("ColumnBinding", Some(s)) => {
            attribute.source_column = text(s, "ColumnID").map(|c| decode_xml_name(&c));
        }
        _ => {}
    }
    attribute
}

fn parse_end(node: Node<'_, '_>) -> RelationshipEnd {
    RelationshipEnd {
        dimension: text(node, "DimensionID").unwrap_or_default(),
        attribute: child(node, "Attributes")
            .and_then(|a| child(a, "Attribute"))
            .and_then(|a| text(a, "AttributeID"))
            .unwrap_or_default(),
        multiplicity: text(node, "Multiplicity"),
    }
}

fn parse_dimension(text_doc: &str, path: &str) -> Result<Dimension, AbfError> {
    const WHAT: &str = "dimension definition";
    let doc = xml::parse(text_doc, WHAT)?;
    let node = object(&doc, "Dimension").ok_or(AbfError::MissingField {
        what: WHAT,
        element: "Dimension",
    })?;
    let mut dim = Dimension {
        id: text(node, "ID").unwrap_or_default(),
        name: text(node, "Name").unwrap_or_default(),
        description: text(node, "Description"),
        annotations: parse_annotations(node),
        path: path.to_owned(),
        ..Dimension::default()
    };
    if let Some(attrs) = child(node, "Attributes") {
        dim.attributes = children(attrs, "Attribute").map(parse_attribute).collect();
    }
    if let Some(hierarchies) = child(node, "Hierarchies") {
        for h in children(hierarchies, "Hierarchy") {
            let levels = child(h, "Levels")
                .map(|l| {
                    children(l, "Level")
                        .map(|level| {
                            (
                                text(level, "Name").unwrap_or_default(),
                                text(level, "SourceAttributeID").unwrap_or_default(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            dim.hierarchies.push(XmlHierarchy {
                name: text(h, "Name").unwrap_or_default(),
                description: text(h, "Description"),
                visible: flag(h, "Visible").unwrap_or(true),
                display_folder: text(h, "DisplayFolder"),
                levels,
            });
        }
    }
    if let Some(relationships) = child(node, "Relationships") {
        for r in children(relationships, "Relationship") {
            let (Some(from), Some(to)) = (
                child(r, "FromRelationshipEnd"),
                child(r, "ToRelationshipEnd"),
            ) else {
                continue;
            };
            dim.relationships.push(XmlRelationship {
                id: text(r, "ID").unwrap_or_default(),
                visible: flag(r, "Visible").unwrap_or(true),
                from: parse_end(from),
                to: parse_end(to),
            });
        }
    }
    Ok(dim)
}

fn parse_cube(text_doc: &str) -> Result<(String, Vec<CubeDimension>), AbfError> {
    const WHAT: &str = "cube definition";
    let doc = xml::parse(text_doc, WHAT)?;
    let cube = object(&doc, "Cube").ok_or(AbfError::MissingField {
        what: WHAT,
        element: "Cube",
    })?;
    let name = text(cube, "Name").unwrap_or_else(|| "Model".to_owned());
    let dims = child(cube, "Dimensions")
        .map(|d| {
            children(d, "Dimension")
                .map(|n| {
                    let id = text(n, "DimensionID")
                        .or_else(|| text(n, "ID"))
                        .unwrap_or_default();
                    CubeDimension {
                        name: text(n, "Name").unwrap_or_else(|| id.clone()),
                        id,
                        visible: flag(n, "Visible").unwrap_or(true),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((name, dims))
}

fn parse_partition(text_doc: &str, path: &str) -> Result<XmlPartition, AbfError> {
    const WHAT: &str = "partition definition";
    let doc = xml::parse(text_doc, WHAT)?;
    let node = object(&doc, "Partition").ok_or(AbfError::MissingField {
        what: WHAT,
        element: "Partition",
    })?;
    let source = child(node, "Source");
    let group = object(&doc, "ParentObject").and_then(|p| text(p, "MeasureGroupID"));
    Ok(XmlPartition {
        group,
        name: text(node, "Name")
            .or_else(|| text(node, "ID"))
            .unwrap_or_default(),
        query: source.and_then(|s| text(s, "QueryDefinition")),
        data_source: source.and_then(|s| text(s, "DataSourceID")),
        path: path.to_owned(),
    })
}

/// `(ID, Name)` of a data source; the connection string is never read.
fn parse_data_source(text_doc: &str) -> Result<Option<(String, String)>, AbfError> {
    const WHAT: &str = "data source definition";
    let doc = xml::parse(text_doc, WHAT)?;
    Ok(object(&doc, "DataSource").and_then(|n| {
        let id = text(n, "ID")?;
        let name = text(n, "Name").unwrap_or_else(|| id.clone());
        Some((id, name))
    }))
}

/// `(table, name, expression)` of every `CREATE MEASURE` command of an MDX script.
fn parse_mdx_script(text_doc: &str) -> Result<Vec<(String, String, String)>, AbfError> {
    const WHAT: &str = "MDX script";
    let doc = xml::parse(text_doc, WHAT)?;
    let mut out = Vec::new();
    for command in doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "Command")
    {
        let Some(body) = text(command, "Text") else {
            continue;
        };
        let annotations = parse_annotations(command);
        let annotated = |key: &str| {
            annotations
                .iter()
                .find(|a| a.name == key)
                .map(|a| a.value.clone())
                .filter(|v| !v.is_empty())
        };
        let measures = parse_create_measures(&body);
        let single = measures.len() == 1;
        for (table, name, expression) in measures {
            let (table, name) = if single {
                (
                    annotated("Table").unwrap_or(table),
                    annotated("FullName").unwrap_or(name),
                )
            } else {
                (table, name)
            };
            out.push((table, name, expression));
        }
    }
    Ok(out)
}

fn find_ci(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let hay = haystack.as_bytes();
    let needle = needle.as_bytes();
    (from..=hay.len().checked_sub(needle.len())?).find(|&i| {
        hay.get(i..i + needle.len())
            .is_some_and(|w| w.eq_ignore_ascii_case(needle))
    })
}

/// Reads a delimited name (`'…'` with `''`, or `[…]` with `]]`) at the start of `s`;
/// returns the unescaped name and the rest.
fn delimited(s: &str, open: char, close: char) -> Option<(String, &str)> {
    let mut chars = s.strip_prefix(open)?.char_indices().peekable();
    let body = s.get(open.len_utf8()..)?;
    let mut name = String::new();
    while let Some((i, c)) = chars.next() {
        if c == close {
            if chars.peek().map(|&(_, n)| n) == Some(close) {
                chars.next();
                name.push(close);
                continue;
            }
            return Some((name, body.get(i + close.len_utf8()..)?));
        }
        name.push(c);
    }
    None
}

/// Splits `CREATE MEASURE 'Table'[Name] = <DAX>;` statements.
pub(crate) fn parse_create_measures(text: &str) -> Vec<(String, String, String)> {
    const KEYWORD: &str = "CREATE MEASURE";
    let mut starts = Vec::new();
    let mut from = 0;
    while let Some(i) = find_ci(text, KEYWORD, from) {
        starts.push(i);
        from = i + KEYWORD.len();
    }
    let mut out = Vec::new();
    for (k, &start) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(text.len());
        let Some(statement) = text.get(start + KEYWORD.len()..end) else {
            continue;
        };
        let rest = statement.trim_start();
        let (table, rest) = if rest.starts_with('\'') {
            match delimited(rest, '\'', '\'') {
                Some(t) => t,
                None => continue,
            }
        } else {
            let cut = rest.find('[').unwrap_or(rest.len());
            (rest[..cut].trim().to_owned(), &rest[cut..])
        };
        let Some((name, rest)) = delimited(rest.trim_start(), '[', ']') else {
            continue;
        };
        let Some(expression) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let expression = expression.trim();
        let expression = expression
            .strip_suffix(';')
            .unwrap_or(expression)
            .trim_end();
        out.push((table, name, expression.to_owned()));
    }
    out
}

fn data_type(assl: Option<&str>) -> DataType {
    match assl.unwrap_or("Empty") {
        "WChar" | "Char" | "String" => DataType::String,
        "BigInt" | "Integer" | "SmallInt" | "TinyInt" | "UnsignedBigInt" | "UnsignedInt"
        | "UnsignedSmallInt" | "UnsignedTinyInt" => DataType::Int64,
        "Double" | "Single" => DataType::Double,
        "Currency" => DataType::Decimal,
        "Date" => DataType::DateTime,
        "Boolean" => DataType::Boolean,
        "Binary" => DataType::Binary,
        "Empty" => DataType::Automatic,
        other => DataType::parse(other),
    }
}

fn cardinality(multiplicity: Option<&str>, default: Cardinality) -> Cardinality {
    match multiplicity {
        Some(m) if m.eq_ignore_ascii_case("one") => Cardinality::One,
        Some(m) if m.eq_ignore_ascii_case("many") => Cardinality::Many,
        _ => default,
    }
}

// ---------------------------------------------------------------------------
// Storage (`.tbl.xml`)
// ---------------------------------------------------------------------------

/// Most segments accepted per column partition.
const MAX_SEGMENTS: usize = 1 << 20;

/// The `XMObject` child of a `Member` named `name`.
fn member<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    child(node, "Members")?
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "Member")
        .find(|m| text(*m, "Name").as_deref() == Some(name))
        .and_then(|m| child(m, "XMObject"))
}

/// The `XMObject` children of a `Collection` named `name`.
fn collection<'a, 'i: 'a>(
    node: Node<'a, 'i>,
    name: &'a str,
) -> impl Iterator<Item = Node<'a, 'i>> + 'a {
    child(node, "Collections")
        .into_iter()
        .flat_map(move |c| children(c, "Collection"))
        .filter(move |c| text(*c, "Name").as_deref() == Some(name))
        .flat_map(|c| children(c, "XMObject"))
}

fn props<'a, 'i>(node: Node<'a, 'i>) -> Option<Node<'a, 'i>> {
    child(node, "Properties")
}

fn prop_i64(node: Option<Node<'_, '_>>, name: &str) -> Option<i64> {
    text(props(node?)?, name)?.parse().ok()
}

fn prop_u64(node: Option<Node<'_, '_>>, name: &str) -> Option<u64> {
    text(props(node?)?, name)?.parse().ok()
}

fn prop_u32(node: Option<Node<'_, '_>>, name: &str) -> Option<u32> {
    prop_i64(node, name).and_then(|v| u32::try_from(v).ok())
}

fn prop_f64(node: Option<Node<'_, '_>>, name: &str) -> Option<f64> {
    text(props(node?)?, name)?.parse().ok()
}

fn prop_bool(node: Option<Node<'_, '_>>, name: &str) -> Option<bool> {
    props(node?).and_then(|p| flag(p, name))
}

fn class<'a>(node: Node<'a, '_>) -> &'a str {
    node.attribute("class").unwrap_or_default()
}

/// `XMRENoSplitCompressionInfo<6>` → 6.
fn no_split_width(class: &str) -> Option<u32> {
    let inner = class.strip_prefix("XMRENoSplitCompressionInfo<")?;
    inner
        .split('>')
        .next()?
        .trim()
        .parse()
        .ok()
        .filter(|n| (1..=64).contains(n))
}

/// Compression class code of a `CompressionInfo` object (the `.idfmeta` codes).
fn compression_code(class: &str) -> u32 {
    if class.starts_with("XMHybridRLECompressionInfo") {
        HYBRID_RLE_CLASS
    } else if let Some(n) = no_split_width(class) {
        NO_SPLIT_CLASS_BASE + n
    } else if class.starts_with("XM123CompressionInfo") {
        XM123_CLASS
    } else if class.starts_with("XMREGeneralCompressionInfo") {
        GENERAL_CLASS
    } else {
        0
    }
}

fn segment_stats(node: Option<Node<'_, '_>>) -> SegmentStats {
    SegmentStats {
        distinct_states: prop_u64(node, "DistinctStates").unwrap_or(0),
        min_data_id: prop_u32(node, "MinDataID").unwrap_or(0),
        max_data_id: prop_u32(node, "MaxDataID").unwrap_or(0),
        original_min_segment_data_id: prop_u32(node, "OriginalMinSegmentDataID").unwrap_or(0),
        rle_sort_order: prop_i64(node, "RLESortOrder").unwrap_or(-1),
        row_count: prop_u64(node, "RowCount").unwrap_or(0),
        has_nulls: prop_bool(node, "HasNulls").unwrap_or(false),
        rle_runs: prop_u64(node, "RLERuns").unwrap_or(0),
        others_rle_runs: prop_u64(node, "OthersRLERuns").unwrap_or(0),
    }
}

/// One `XMColumnSegment` → the `.idfmeta` view of it.
fn segment_meta(node: Node<'_, '_>) -> SegmentMeta {
    let info = member(node, "CompressionInfo");
    let info_class = info.map(class).unwrap_or_default();
    let compression_class = compression_code(info_class);
    let (rle, sub_compression_class) = if compression_class == HYBRID_RLE_CLASS {
        let rle = info.and_then(|i| member(i, "RLECompression"));
        let sub = info
            .and_then(|i| member(i, "SubCompression"))
            .map(|s| compression_code(class(s)))
            .unwrap_or(0);
        (
            Some(RleFacet {
                bookmark_bits: prop_u64(rle, "BookmarkBits").unwrap_or(0),
                storage_alloc_size: prop_u64(rle, "StorageAllocSize").unwrap_or(0),
                storage_used_size: prop_u64(rle, "StorageUsedSize").unwrap_or(0),
                segment_needs_resizing: prop_bool(rle, "SegmentNeedsResizing").unwrap_or(false),
            }),
            sub,
        )
    } else {
        (None, 0)
    };
    let subsegment = member(node, "SubSegment").map(|s| SubSegmentMeta {
        records: prop_u64(Some(s), "Records").unwrap_or(0),
        base_id: 0,
    });
    SegmentMeta {
        records: prop_u64(Some(node), "Records").unwrap_or(0),
        base_id: 0,
        compression_class,
        sub_compression_class,
        rle,
        first_run_value: 0,
        stats: segment_stats(member(node, "ColumnSegmentStats")),
        subsegment,
    }
}

/// Physical type of a dictionary class (`XMHashDataDictionary<XM_String>` → string).
fn dictionary_type(class: &str) -> Option<(DataType, i64)> {
    if class.contains("XM_String") {
        Some((DataType::String, 2))
    } else if class.contains("XM_Long") {
        Some((DataType::Int64, 6))
    } else if class.contains("XM_Real") {
        Some((DataType::Double, 8))
    } else {
        None
    }
}

fn type_code(t: &DataType) -> i64 {
    match t {
        DataType::String => 2,
        DataType::Int64 => 6,
        DataType::Double => 8,
        DataType::DateTime => 9,
        DataType::Decimal => 10,
        DataType::Boolean => 11,
        DataType::Binary => 17,
        DataType::Variant => 20,
        _ => 1,
    }
}

fn file_path(archive: &AbfArchive, name: &str) -> Result<String, AbfError> {
    archive
        .find_file_name(name)
        .map(|f| f.logical_path.clone())
        .ok_or_else(|| AbfError::FileNotFound(name.to_owned()))
}

/// Storage of one column from its `XMRawColumn` object, named `name` with the model's
/// `logical` type. Returns the type inferred from the dictionary when `logical` is
/// automatic (calculated columns).
fn column_storage(
    archive: &AbfArchive,
    node: Node<'_, '_>,
    name: &str,
    kind: ColumnKindCode,
    logical: &DataType,
) -> Result<(ColumnStorage, Option<DataType>), AbfError> {
    let stats = member(node, "ColumnStats");
    let mut segments: Vec<SegmentMeta> = collection(node, "Segments").map(segment_meta).collect();
    if segments.len() > MAX_SEGMENTS {
        return Err(AbfError::LimitExceeded {
            what: "segments of a column",
            value: segments.len() as u64,
            limit: MAX_SEGMENTS as u64,
        });
    }
    let mut dictionary = None;
    let mut physical = None;
    let mut partitions = Vec::new();
    let objects = child(node, "DataObjects")
        .into_iter()
        .flat_map(|d| children(d, "DataObject"))
        .filter_map(|d| child(d, "XMObject"));
    for object in objects {
        let object_class = class(object);
        if object_class.starts_with("XMRawColumnPartitionDataObject") {
            let count = prop_u64(Some(object), "SegmentCount")
                .and_then(|c| usize::try_from(c).ok())
                .unwrap_or(segments.len())
                .min(segments.len());
            let taken: Vec<SegmentMeta> = segments.drain(..count).collect();
            let idf = object
                .attribute("name")
                .map(|n| file_path(archive, n))
                .transpose()?;
            let meta = ColumnPartitionMeta {
                segments: taken,
                data_objects: None,
            };
            partitions.push(ColumnPartitionFiles {
                partition_id: prop_i64(Some(object), "Partition").unwrap_or(0),
                partition_storage_id: 0,
                storage_position: i64::try_from(partitions.len()).unwrap_or(0),
                record_count: meta.records(),
                segment_count: Some(meta.segments.len() as u64),
                data_version: prop_i64(Some(object), "DataVersion"),
                state: None,
                idf,
                idfmeta: None,
                meta: Some(meta),
            });
        } else if object_class.starts_with("XMHashDataDictionary")
            || object_class.starts_with("XMValueDataDictionary")
        {
            let hash = object_class.starts_with("XMHashDataDictionary");
            let kind = dictionary_type(object_class);
            physical = kind.as_ref().map(|(t, _)| t.clone());
            dictionary = Some(DictionaryStorage {
                storage_id: 0,
                encoding: if hash {
                    DictionaryEncoding::Hash
                } else {
                    DictionaryEncoding::Value
                },
                data_type_code: kind.map_or(19, |(_, code)| code),
                data_version: prop_i64(Some(object), "DataVersion"),
                base_id: if hash {
                    2
                } else {
                    prop_i64(Some(object), "BaseId").unwrap_or(0)
                },
                magnitude: if hash {
                    0.0
                } else {
                    prop_f64(Some(object), "Magnitude").unwrap_or(1.0)
                },
                last_id: prop_i64(Some(object), "LastId"),
                is_nullable: prop_bool(Some(object), "Nullable").unwrap_or(true),
                is_unique: prop_bool(Some(object), "Unique").unwrap_or(false),
                is_operating_on_32: prop_bool(Some(object), "OperatingOn32").unwrap_or(false),
                dictionary_flags: prop_i64(Some(object), "DictionaryFlags").unwrap_or(0),
                size: None,
                path: if hash {
                    object
                        .attribute("name")
                        .map(|n| file_path(archive, n))
                        .transpose()?
                } else {
                    None
                },
            });
        }
    }
    let data_type = match (logical, &physical) {
        (DataType::Automatic | DataType::Unknown(_), Some(p)) => p.clone(),
        (t, _) => t.clone(),
    };
    let inferred = matches!(logical, DataType::Automatic).then(|| data_type.clone());
    Ok((
        ColumnStorage {
            column_id: 0,
            name: name.to_owned(),
            kind,
            data_type_code: type_code(&data_type),
            data_type,
            storage_id: 0,
            storage_name: node.attribute("name").map(str::to_owned),
            storage_position: 0,
            settings: prop_i64(Some(node), "Settings"),
            column_flags: prop_i64(Some(node), "ColumnFlags"),
            collation: text(props(node).unwrap_or(node), "Collation"),
            locale: prop_i64(Some(node), "Locale"),
            statistics: ColumnStatistics {
                distinct_states: prop_i64(stats, "DistinctStates"),
                min_data_id: prop_i64(stats, "MinDataID"),
                max_data_id: prop_i64(stats, "MaxDataID"),
                original_min_segment_data_id: prop_i64(stats, "OriginalMinSegmentDataID"),
                rle_sort_order: prop_i64(stats, "RLESortOrder"),
                row_count: prop_i64(stats, "RowCount"),
                has_nulls: prop_bool(stats, "HasNulls"),
                rle_runs: prop_i64(stats, "RLERuns"),
                others_rle_runs: prop_i64(stats, "OthersRLERuns"),
                usage: prop_i64(stats, "Usage"),
                db_type: prop_i64(stats, "DBType"),
                xm_type: prop_i64(stats, "XMType"),
                compression_type: prop_i64(stats, "CompressionType"),
                compression_param: prop_i64(stats, "CompressionParam"),
                encoding_hint: prop_i64(stats, "EncodingHint"),
            },
            dictionary,
            attribute_hierarchy: None,
            partitions,
        },
        inferred,
    ))
}

/// The storage of `table` (a model table built from dimension `dim`) from its
/// `.tbl.xml`. Fills in the type of calculated columns from their dictionaries.
fn table_storage(
    archive: &AbfArchive,
    dim: &Dimension,
    table: &mut Table,
) -> Result<TableStorage, AbfError> {
    const WHAT: &str = "table storage";
    let prefix = format!("{}.", dim.id);
    let file = files_of_kind(archive, "tbl")
        .into_iter()
        .find(|f| {
            let name = f.file_name();
            name.starts_with(&prefix) && object_id(name, "tbl").as_deref() == Some(dim.id.as_str())
        })
        .ok_or(AbfError::MissingField {
            what: WHAT,
            element: "XMSimpleTable (.tbl.xml)",
        })?;
    let text_doc = read_text(archive, file, WHAT)?;
    let doc = xml::parse(&text_doc, WHAT)?;
    let root = doc
        .descendants()
        .find(|n| n.is_element() && n.tag_name().name() == "XMObject")
        .ok_or(AbfError::MissingField {
            what: WHAT,
            element: "XMObject",
        })?;
    let row_count = member(root, "SegmentMap")
        .map(|m| {
            collection(m, "Partitions")
                .filter_map(|p| prop_u64(Some(p), "Records"))
                .sum()
        })
        .unwrap_or(0);
    let mut columns = Vec::new();
    for node in collection(root, "Columns") {
        let Some(id) = node.attribute("name") else {
            continue;
        };
        let attribute = dim.attributes.iter().find(|a| a.id == id);
        let row_number = attribute.is_some_and(|a| a.row_number);
        let (name, kind, logical) = match attribute {
            Some(a) if a.row_number => (a.name.clone(), ColumnKindCode::RowNumber, DataType::Int64),
            Some(a) => {
                let column = table.columns.iter().find(|c| c.name == a.name);
                let kind = if a.expression.is_some() {
                    ColumnKindCode::Calculated
                } else {
                    ColumnKindCode::Data
                };
                (
                    a.name.clone(),
                    kind,
                    column.map_or(DataType::Automatic, |c| c.data_type.clone()),
                )
            }
            None => continue,
        };
        let (storage, inferred) = column_storage(archive, node, &name, kind, &logical)?;
        if let (Some(t), false) = (inferred, row_number)
            && let Some(column) = table.columns.iter_mut().find(|c| c.name == name)
        {
            column.data_type = t;
        }
        columns.push(storage);
    }
    Ok(TableStorage {
        table_id: 0,
        name: table.name.clone(),
        storage_name: Some(dim.id.clone()),
        folder: None,
        system_flags: 0,
        is_internal: false,
        row_count,
        partitions: Vec::new(),
        columns,
    })
}

// ---------------------------------------------------------------------------
// Loader
// ---------------------------------------------------------------------------

struct Loader<'a> {
    archive: &'a AbfArchive,
    options: &'a ModelOptions,
    diagnostics: Vec<Diagnostic>,
}

impl Loader<'_> {
    /// `<backup>#/<xml file>`, or the XML file itself when no backup path was given.
    fn origin(&self, file: &str) -> Option<Origin> {
        let base = self.options.origin_path.as_ref()?;
        Some(if base == crate::abf::METADATA_FILE_NAME {
            Origin::file(file)
        } else {
            Origin::file(base.clone()).with_pointer(format!("/{file}"))
        })
    }

    fn skipped(&mut self, file: &AbfFile, err: &AbfError) {
        tracing::warn!(file = %file.logical_path, %err, "XML metadata part skipped");
        let mut d = Diagnostic::warning(
            codes::XML_METADATA_PART,
            format!("{}: {err}; skipped", file.logical_path),
        );
        d.origin = self.origin(&file.logical_path);
        self.diagnostics.push(d);
    }

    fn dimensions(&mut self) -> HashMap<String, Dimension> {
        let files = files_of_kind(self.archive, "dim");
        let mut out = HashMap::new();
        for file in files {
            let parsed = read_text(self.archive, file, "dimension definition")
                .and_then(|t| parse_dimension(&t, &file.logical_path));
            match parsed {
                Ok(mut dim) => {
                    if dim.id.is_empty() {
                        dim.id = object_id(&file.logical_path, "dim").unwrap_or_default();
                    }
                    out.insert(dim.id.clone(), dim);
                }
                Err(err) => self.skipped(file, &err),
            }
        }
        out
    }

    fn partitions(&mut self) -> HashMap<String, Vec<XmlPartition>> {
        let files = files_of_kind(self.archive, "prt");
        let mut out: HashMap<String, Vec<XmlPartition>> = HashMap::new();
        for file in files {
            match read_text(self.archive, file, "partition definition")
                .and_then(|t| parse_partition(&t, &file.logical_path))
            {
                Ok(p) => {
                    let group = p.group.clone().or_else(|| {
                        file.logical_path
                            .rsplit('/')
                            .nth(1)
                            .and_then(|folder| folder_object_id(folder, "det"))
                    });
                    if let Some(group) = group {
                        out.entry(group).or_default().push(p);
                    }
                }
                Err(err) => self.skipped(file, &err),
            }
        }
        out
    }

    fn data_sources(&mut self) -> HashMap<String, String> {
        let files = files_of_kind(self.archive, "ds");
        let mut out = HashMap::new();
        for file in files {
            match read_text(self.archive, file, "data source definition")
                .and_then(|t| parse_data_source(&t))
            {
                Ok(Some((id, name))) => {
                    out.insert(id, name);
                }
                Ok(None) => {}
                Err(err) => self.skipped(file, &err),
            }
        }
        out
    }

    fn measures(&mut self) -> Vec<(String, String, String)> {
        let files = files_of_kind(self.archive, "scr");
        let mut out = Vec::new();
        for file in files {
            match read_text(self.archive, file, "MDX script").and_then(|t| parse_mdx_script(&t)) {
                Ok(m) => out.extend(m),
                Err(err) => self.skipped(file, &err),
            }
        }
        out
    }
}

fn build_table(
    cube_dim: &CubeDimension,
    dim: Option<&Dimension>,
    partitions: &[XmlPartition],
    data_sources: &HashMap<String, String>,
    options: &ModelOptions,
    origin: impl Fn(&str) -> Option<Origin>,
) -> Table {
    let mut table = Table {
        name: cube_dim.name.clone(),
        is_hidden: !cube_dim.visible,
        ..Table::default()
    };
    table.extra.insert(
        "vertipaq.dimensionId".into(),
        PropValue::Text(cube_dim.id.clone()),
    );
    let Some(dim) = dim else {
        return table;
    };
    table.description = dim.description.clone();
    table.annotations = dim
        .annotations
        .iter()
        .filter(|a| !a.value.is_empty())
        .cloned()
        .collect();
    table.origin = origin(&dim.path);
    let names: HashMap<&str, &str> = dim
        .attributes
        .iter()
        .map(|a| (a.id.as_str(), a.name.as_str()))
        .collect();
    for attr in &dim.attributes {
        if attr.row_number && !options.include_row_number_columns {
            continue;
        }
        let kind = if attr.row_number {
            ColumnKind::RowNumber
        } else if let Some(expression) = &attr.expression {
            ColumnKind::Calculated {
                expression: expression.clone(),
            }
        } else {
            ColumnKind::Data {
                source_column: attr
                    .source_column
                    .clone()
                    .unwrap_or_else(|| attr.name.clone()),
            }
        };
        let mut column = Column {
            name: attr.name.clone(),
            description: attr.description.clone(),
            data_type: data_type(attr.data_type.as_deref()),
            kind,
            is_hidden: !attr.visible || attr.row_number,
            format_string: attr.format_string.clone(),
            display_folder: attr.display_folder.clone(),
            sort_by_column: attr
                .order_by_attribute
                .as_deref()
                .and_then(|id| names.get(id))
                .map(|n| (*n).to_owned()),
            ..Column::default()
        };
        if attr.id != attr.name {
            column.extra.insert(
                "vertipaq.attributeId".into(),
                PropValue::Text(attr.id.clone()),
            );
        }
        table.columns.push(column);
    }
    for h in &dim.hierarchies {
        table.hierarchies.push(Hierarchy {
            name: h.name.clone(),
            description: h.description.clone(),
            is_hidden: !h.visible,
            display_folder: h.display_folder.clone(),
            levels: h
                .levels
                .iter()
                .enumerate()
                .map(|(i, (name, attr))| Level {
                    name: name.clone(),
                    ordinal: i64::try_from(i).ok(),
                    column: names.get(attr.as_str()).copied().unwrap_or(attr).to_owned(),
                    ..Level::default()
                })
                .collect(),
            ..Hierarchy::default()
        });
    }
    for p in partitions {
        let source = match &p.query {
            Some(query) => PartitionSource::Query {
                query: query.clone(),
                data_source: p
                    .data_source
                    .as_ref()
                    .map(|id| data_sources.get(id).cloned().unwrap_or_else(|| id.clone())),
            },
            None => PartitionSource::Unknown {
                kind: "legacy".into(),
                props: Default::default(),
            },
        };
        table.partitions.push(Partition {
            name: p.name.clone(),
            mode: PartitionMode::Import,
            source,
            origin: origin(&p.path),
            ..Partition::default()
        });
    }
    table
}

/// Reads a model described by legacy XML metadata. The storage map is empty (see the
/// module docs).
pub(crate) fn load_xml_model(
    archive: &AbfArchive,
    options: &ModelOptions,
) -> Result<LoadedModel, MetadataError> {
    let cube = cube_file(archive).ok_or(MetadataError::MissingMetadata)?;
    let (model_name, mut cube_dims) = parse_cube(&read_text(archive, cube, "cube definition")?)?;
    let mut loader = Loader {
        archive,
        options,
        diagnostics: Vec::new(),
    };
    let dims = loader.dimensions();
    for cube_dim in &mut cube_dims {
        if let Some(dim) = dims.get(&cube_dim.id)
            && cube_dim.name == cube_dim.id
            && !dim.name.is_empty()
        {
            cube_dim.name = dim.name.clone();
        }
    }
    let partitions = loader.partitions();
    let data_sources = loader.data_sources();
    let measures = loader.measures();

    let mut model = Model {
        name: model_name,
        ..Model::default()
    };
    model.extra.insert(
        "vertipaq.metadataFormat".into(),
        PropValue::Text("xml".into()),
    );
    let empty = Vec::new();
    let mut storage = StorageMap::default();
    let mut unmapped = Vec::new();
    for cube_dim in &cube_dims {
        let dim = dims.get(&cube_dim.id);
        let mut table = build_table(
            cube_dim,
            dim,
            partitions.get(&cube_dim.id).unwrap_or(&empty),
            &data_sources,
            options,
            |file| loader.origin(file),
        );
        match dim.map(|d| table_storage(archive, d, &mut table)) {
            Some(Ok(t)) => storage.tables.push(t),
            Some(Err(err)) => {
                tracing::warn!(table = %table.name, %err, "table storage not mapped");
                loader.diagnostics.push(Diagnostic::warning(
                    codes::XML_METADATA_PART,
                    format!(
                        "table '{}': storage not readable ({err}); its data cannot be decoded",
                        table.name
                    ),
                ));
                unmapped.push(table.name.clone());
            }
            None => unmapped.push(table.name.clone()),
        }
        model.tables.push(table);
    }

    for (table, name, expression) in measures {
        let Some(t) = model.table_mut(&table) else {
            loader.diagnostics.push(Diagnostic::warning(
                codes::XML_METADATA_PART,
                format!("measure [{name}] belongs to unknown table '{table}'; skipped"),
            ));
            continue;
        };
        t.measures.push(Measure::new(name, expression));
    }

    let table_of: HashMap<&str, &str> = cube_dims
        .iter()
        .map(|d| (d.id.as_str(), d.name.as_str()))
        .collect();
    let column_of = |dim: &str, attr: &str| -> String {
        dims.get(dim)
            .and_then(|d| d.attributes.iter().find(|a| a.id == attr))
            .map_or_else(|| attr.to_owned(), |a| a.name.clone())
    };
    let mut dim_ids: Vec<&String> = dims.keys().collect();
    dim_ids.sort_by_key(|id| cube_dims.iter().position(|c| &&c.id == id));
    for id in dim_ids {
        let Some(dim) = dims.get(id) else { continue };
        for r in &dim.relationships {
            let (Some(from_table), Some(to_table)) = (
                table_of.get(r.from.dimension.as_str()),
                table_of.get(r.to.dimension.as_str()),
            ) else {
                continue;
            };
            let mut rel = Relationship {
                name: r.id.clone(),
                from: ColumnRef::new(*from_table, column_of(&r.from.dimension, &r.from.attribute)),
                to: ColumnRef::new(*to_table, column_of(&r.to.dimension, &r.to.attribute)),
                from_cardinality: cardinality(r.from.multiplicity.as_deref(), Cardinality::Many),
                to_cardinality: cardinality(r.to.multiplicity.as_deref(), Cardinality::One),
                is_active: r.visible,
                ..Relationship::default()
            };
            rel.origin = loader.origin(&dim.path);
            model.relationships.push(rel);
        }
    }

    model.database = super::database::database_from_archive(archive);
    let mut diagnostics = loader.diagnostics;
    let note = if unmapped.is_empty() {
        Diagnostic::info(
            codes::XML_METADATA,
            "legacy XML metadata (compatibility level 1100/1103, e.g. Power Pivot): schema and \
             VertiPaq storage read from the XML object definitions",
        )
    } else {
        Diagnostic::warning(
            codes::XML_METADATA,
            format!(
                "legacy XML metadata (compatibility level 1100/1103, e.g. Power Pivot): the schema \
                 was read, but the storage of {} table(s) is not mapped, so their data cannot be \
                 decoded: {}",
                unmapped.len(),
                unmapped.join(", ")
            ),
        )
    };
    diagnostics.insert(0, note);
    Ok(LoadedModel {
        model,
        storage,
        schema_version: None,
        format: MetadataFormat::Xml,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_names_and_object_ids_decode() {
        assert_eq!(
            decode_xml_name("Sub_x0020_Category_x0020_ID"),
            "Sub Category ID"
        );
        assert_eq!(
            decode_xml_name("Downtime_x0020__x0028_min_x0029_"),
            "Downtime (min)"
        );
        assert_eq!(decode_xml_name("a_xZZZZ_b_x"), "a_xZZZZ_b_x");
        assert_eq!(
            object_id("Date_29d5.fe95.43.dim.xml", "dim").as_deref(),
            Some("Date_29d5.fe95")
        );
        assert_eq!(object_id("x.dim.xml", "dim"), None);
        assert_eq!(
            folder_object_id("Metrics_1.208.det", "det").as_deref(),
            Some("Metrics_1")
        );
    }

    #[test]
    fn create_measure_statements_split() {
        let text = "--comment\nCREATE MEASURE 'Sales ''EU'''[Total [x]]]=SUM(Sales[Amount]);\n\
                    create measure Sales[Avg] = AVERAGE(Sales[Amount]) ;";
        let m = parse_create_measures(text);
        assert_eq!(
            m,
            vec![
                (
                    "Sales 'EU'".to_owned(),
                    "Total [x]".to_owned(),
                    "SUM(Sales[Amount])".to_owned()
                ),
                (
                    "Sales".to_owned(),
                    "Avg".to_owned(),
                    "AVERAGE(Sales[Amount])".to_owned()
                ),
            ]
        );
        assert!(parse_create_measures("CREATE MEASURE 'unterminated").is_empty());
        assert!(parse_create_measures("CREATE MEMBER CURRENTCUBE.x AS 1").is_empty());
    }
}
