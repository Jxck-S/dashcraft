//! Semantic diff of two documents (M8.3): the model and report objects
//! added, removed and changed between them.
//!
//! Objects are matched by name (tables, columns, measures, … case-insensitive
//! like the model; pages and visuals by their PBIR names), not by file or
//! line, so file layout, formatting and object order do not count. They are
//! compared on their serialized properties without the document-local parts:
//! object ids and source origins. A changed object lists the properties that
//! differ, with both values; an added or removed one lists all of its
//! properties, children included (its children are not listed separately).
//!
//! Each object also carries a text [`Snippet`] per side: model objects as
//! TMDL, written by the TMDL writer from each side's model (so formatting
//! never differs, only content), report objects as their properties in
//! JSON. A changed table or report object leaves out the children compared
//! on their own; an added or removed one keeps them. [`diff_projects`]
//! shows report objects as their PBIR files instead (`report.json`,
//! `page.json`, `visual.json`, as a save writes them) where both sides have
//! one.

use std::collections::HashMap;

use pbi_model::names::name_key;
use pbi_model::{
    CalculationItem, Column, Culture, Database, Document, Hierarchy, Measure, Model,
    NamedExpression, Origin, Page, Partition, Perspective, QueryGroup, Relationship, Report,
    ReportMeasure, Role, Table, Visual,
};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::container::path::join;
use crate::reader::{OpenedDocument, ReportSource};

/// The kind of a compared object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffKind {
    Model,
    Database,
    Table,
    Column,
    Measure,
    Partition,
    Hierarchy,
    CalculationItem,
    Relationship,
    Expression,
    QueryGroup,
    Role,
    Culture,
    Perspective,
    Report,
    ReportMeasure,
    Page,
    Visual,
}

impl DiffKind {
    /// The camelCase name used in JSON outputs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DiffKind::Model => "model",
            DiffKind::Database => "database",
            DiffKind::Table => "table",
            DiffKind::Column => "column",
            DiffKind::Measure => "measure",
            DiffKind::Partition => "partition",
            DiffKind::Hierarchy => "hierarchy",
            DiffKind::CalculationItem => "calculationItem",
            DiffKind::Relationship => "relationship",
            DiffKind::Expression => "expression",
            DiffKind::QueryGroup => "queryGroup",
            DiffKind::Role => "role",
            DiffKind::Culture => "culture",
            DiffKind::Perspective => "perspective",
            DiffKind::Report => "report",
            DiffKind::ReportMeasure => "reportMeasure",
            DiffKind::Page => "page",
            DiffKind::Visual => "visual",
        }
    }
}

/// How an object differs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffChange {
    /// Only in the second document.
    Added,
    /// Only in the first document.
    Removed,
    /// In both, with different properties.
    Changed,
}

impl DiffChange {
    /// The camelCase name used in JSON outputs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            DiffChange::Added => "added",
            DiffChange::Removed => "removed",
            DiffChange::Changed => "changed",
        }
    }
}

/// One property of a compared object (camelCase, as serialized), with its
/// value on each side; `None` where the property is absent (or the object
/// is).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyDiff {
    pub name: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// One added, removed or changed object.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectDiff {
    pub kind: DiffKind,
    /// A readable path: `Orders`, `Orders[Total Sales]`,
    /// `Orders[Store ID] -> Stores[Store ID]`, `Overview/Executive Summary`,
    /// `Overview/Executive Summary/<visual name>`, …
    pub path: String,
    pub change: DiffChange,
    /// The differing properties (changed), or all of them (added, removed).
    pub properties: Vec<PropertyDiff>,
    /// Where the object is in the first document.
    pub before: Option<Origin>,
    /// Where the object is in the second document.
    pub after: Option<Origin>,
    /// The object as text on each side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<Snippet>,
}

/// The language of a [`Snippet`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SnippetLanguage {
    /// Model objects, as the TMDL writer writes them.
    Tmdl,
    /// Report objects: their properties, pretty-printed.
    Json,
}

/// An object's text on each side of a diff; a side is absent where the
/// object is. Long texts are cut after [`MAX_SNIPPET_LINES`] lines, with a
/// last line saying how many were left out.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub language: SnippetLanguage,
    pub before: Option<String>,
    pub after: Option<String>,
}

/// The most lines a snippet side keeps.
pub const MAX_SNIPPET_LINES: usize = 200;

/// The objects that differ between two documents, in document order
/// (model, its tables with their children, relationships, expressions, …,
/// then the reports with their pages and visuals).
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentDiff {
    pub objects: Vec<ObjectDiff>,
}

impl DocumentDiff {
    /// No object differs.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// The number of objects with this change.
    #[must_use]
    pub fn count(&self, change: DiffChange) -> usize {
        self.objects.iter().filter(|o| o.change == change).count()
    }
}

/// Compares two documents: what changed from `before` to `after`.
#[must_use]
pub fn diff_documents(before: &Document, after: &Document) -> DocumentDiff {
    let mut d = Differ::default();
    d.models(before.model.as_ref(), after.model.as_ref());
    for (b, a) in pair_by(&before.reports, &after.reports, |r| name_key(&r.name)) {
        d.report(b, a);
    }
    let Differ { mut out, keys } = d;
    if keys.iter().any(Option::is_some) {
        let b = before.model.as_ref().map(TmdlIndex::of).unwrap_or_default();
        let a = after.model.as_ref().map(TmdlIndex::of).unwrap_or_default();
        for (object, key) in out.iter_mut().zip(&keys) {
            let Some(key) = key else {
                continue;
            };
            let own = object.change == DiffChange::Changed;
            let before = if object.change == DiffChange::Added {
                None
            } else {
                b.get(key, own)
            };
            let after = if object.change == DiffChange::Removed {
                None
            } else {
                a.get(key, own)
            };
            if before.is_some() || after.is_some() {
                object.snippet = Some(Snippet {
                    language: SnippetLanguage::Tmdl,
                    before,
                    after,
                });
            }
        }
    }
    DocumentDiff { objects: out }
}

/// [`diff_documents`] of two opened documents, with each report, page and
/// visual shown as its PBIR file (`report.json`, `page.json`,
/// `visual.json`) as a save writes it, where every side the object is on
/// has one; otherwise (legacy layouts, objects without a file of their
/// own) as its properties in JSON. Pass an edited document as
/// [`edited_document`](crate::edited_document) returns it, so the files
/// carry the unsaved edits.
#[must_use]
pub fn diff_projects(before: &OpenedDocument, after: &OpenedDocument) -> DocumentDiff {
    let mut diff = diff_documents(&before.document, &after.document);
    if !diff.objects.iter().any(|o| is_report_kind(o.kind)) {
        return diff;
    }
    let (b, a) = (pbir_texts(before), pbir_texts(after));
    let text = |files: &HashMap<String, String>, origin: Option<&Origin>| {
        origin
            .filter(|o| o.pointer.is_none())
            .and_then(|o| files.get(&o.path))
            .map(|t| capped(t.lines()))
    };
    // `Some(None)`: the object is not on that side; `None`: it has no file.
    let side = |on_side: bool, files: &HashMap<String, String>, origin: Option<&Origin>| {
        if on_side {
            text(files, origin).map(Some)
        } else {
            Some(None)
        }
    };
    for object in diff.objects.iter_mut().filter(|o| is_report_kind(o.kind)) {
        let before = side(
            object.change != DiffChange::Added,
            &b,
            object.before.as_ref(),
        );
        let after = side(
            object.change != DiffChange::Removed,
            &a,
            object.after.as_ref(),
        );
        let (Some(before), Some(after)) = (before, after) else {
            continue;
        };
        if object.change == DiffChange::Changed && before == after {
            continue;
        }
        object.snippet = Some(Snippet {
            language: SnippetLanguage::Json,
            before,
            after,
        });
    }
    diff
}

/// The JSON files of `doc`'s PBIR reports as a save writes them, by path in
/// the container.
fn pbir_texts(doc: &OpenedDocument) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for source in &doc.report_sources {
        let ReportSource::Pbir { folder, report } = source else {
            continue;
        };
        for (path, bytes) in pbi_pbir::write_pbir(report) {
            if path.ends_with(".json") {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                out.insert(join(folder, &path), text);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Snippets

/// Keeps at most [`MAX_SNIPPET_LINES`] lines, without trailing blank ones.
fn capped<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut lines: Vec<&str> = lines.collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    let more = lines.len().saturating_sub(MAX_SNIPPET_LINES);
    lines.truncate(MAX_SNIPPET_LINES);
    let mut text = lines.join("\n");
    if more > 0 {
        text.push_str(&format!("\n… ({more} more lines)"));
    }
    text
}

fn json_snippet(properties: Map<String, Value>) -> String {
    let text = serde_json::to_string_pretty(&Value::Object(properties)).unwrap_or_default();
    capped(text.lines())
}

/// The lookup key of a model object in a [`TmdlIndex`].
fn tmdl_key(kind: DiffKind, name: &str) -> String {
    format!("{}\u{0}{name}", kind.as_str())
}

fn member_key(kind: DiffKind, table: &str, member: &str) -> String {
    tmdl_key(
        kind,
        &format!("{}\u{0}{}", name_key(table), name_key(member)),
    )
}

/// TMDL keywords of a table's children that are compared as objects of
/// their own.
const TABLE_MEMBERS: &[(&str, DiffKind)] = &[
    ("column", DiffKind::Column),
    ("measure", DiffKind::Measure),
    ("partition", DiffKind::Partition),
    ("hierarchy", DiffKind::Hierarchy),
];

/// A node's text, with its `///` description but no other leading trivia,
/// de-indented to its own level.
fn node_text(node: &pbi_tmdl::Node) -> String {
    let mut node = node.clone();
    node.leading.retain(pbi_tmdl::Line::is_description);
    let indent = node.line.indentation().to_owned();
    let text = node.to_string();
    capped(
        text.lines()
            .map(|l| l.strip_prefix(indent.as_str()).unwrap_or(l)),
    )
}

/// A table or model node without the children compared on their own (and
/// without `ref` lines).
fn own_node(node: &pbi_tmdl::Node) -> pbi_tmdl::Node {
    let mut node = node.clone();
    node.children.retain(|c| {
        !c.is_ref()
            && !TABLE_MEMBERS
                .iter()
                .any(|(k, _)| c.keyword().eq_ignore_ascii_case(k))
    });
    for c in &mut node.children {
        if c.keyword().eq_ignore_ascii_case("calculationGroup") {
            c.children
                .retain(|i| !i.keyword().eq_ignore_ascii_case("calculationItem"));
        }
    }
    node
}

/// The TMDL text of every model object, from the TMDL writer's output.
#[derive(Default)]
struct TmdlIndex {
    /// Key → (full text, text without the children compared on their own
    /// where that differs).
    texts: HashMap<String, (String, Option<String>)>,
}

impl TmdlIndex {
    fn of(model: &Model) -> Self {
        let mut texts = HashMap::new();
        for (path, text) in pbi_tmdl::write_model(model, None) {
            let Ok(file) = pbi_tmdl::parse_file(&path, &text) else {
                continue;
            };
            for node in file.nodes.iter().filter(|n| !n.is_ref()) {
                let name = node.name().unwrap_or_default();
                let kind = match node.keyword().to_ascii_lowercase().as_str() {
                    "model" => DiffKind::Model,
                    "database" => DiffKind::Database,
                    "table" => DiffKind::Table,
                    "relationship" => DiffKind::Relationship,
                    "expression" => DiffKind::Expression,
                    "querygroup" => DiffKind::QueryGroup,
                    "role" => DiffKind::Role,
                    "cultureinfo" => DiffKind::Culture,
                    "perspective" => DiffKind::Perspective,
                    _ => continue,
                };
                let key = match kind {
                    DiffKind::Model | DiffKind::Database => tmdl_key(kind, ""),
                    DiffKind::Relationship => tmdl_key(kind, &name),
                    _ => tmdl_key(kind, &name_key(&name)),
                };
                let entry = match kind {
                    DiffKind::Model => (node_text(&own_node(node)), None),
                    DiffKind::Table => {
                        Self::members(&mut texts, &name, node);
                        (node_text(node), Some(node_text(&own_node(node))))
                    }
                    _ => (node_text(node), None),
                };
                texts.insert(key, entry);
            }
        }
        Self { texts }
    }

    fn members(
        texts: &mut HashMap<String, (String, Option<String>)>,
        table: &str,
        node: &pbi_tmdl::Node,
    ) {
        for child in &node.children {
            let keyword = child.keyword();
            let member = TABLE_MEMBERS
                .iter()
                .find(|(k, _)| keyword.eq_ignore_ascii_case(k))
                .map(|(_, kind)| *kind);
            if let Some(kind) = member {
                let name = child.name().unwrap_or_default();
                texts.insert(member_key(kind, table, &name), (node_text(child), None));
            } else if keyword.eq_ignore_ascii_case("calculationGroup") {
                for item in child
                    .children
                    .iter()
                    .filter(|i| i.keyword().eq_ignore_ascii_case("calculationItem"))
                {
                    let name = item.name().unwrap_or_default();
                    texts.insert(
                        member_key(DiffKind::CalculationItem, table, &name),
                        (node_text(item), None),
                    );
                }
            }
        }
    }

    /// The text of an object: without its separately compared children
    /// when `own` is set.
    fn get(&self, key: &str, own: bool) -> Option<String> {
        let (full, without) = self.texts.get(key)?;
        Some(match without {
            Some(without) if own => without.clone(),
            _ => full.clone(),
        })
    }
}

// ---------------------------------------------------------------------------

/// Typed keys whose values are typed model objects (or arrays of them) that
/// carry document-local `id` and `origin` fields. Untyped JSON (`extra`,
/// visual `objects`, …) is never walked, so its own `id` keys stay.
const TYPED_ARRAYS: &[&str] = &[
    "tables",
    "columns",
    "measures",
    "partitions",
    "hierarchies",
    "levels",
    "calculationItems",
    "relationships",
    "expressions",
    "queryGroups",
    "roles",
    "tablePermissions",
    "cultures",
    "perspectives",
    "filters",
    "pages",
    "visuals",
];
const TYPED_OBJECTS: &[&str] = &[
    "database",
    "calculationGroup",
    "multipleOrEmptySelectionExpression",
    "noSelectionExpression",
];

/// Children compared as objects of their own, left out of their parent's
/// properties.
const MODEL_CHILDREN: &[&str] = &[
    "database",
    "tables",
    "relationships",
    "expressions",
    "queryGroups",
    "roles",
    "cultures",
    "perspectives",
];
const TABLE_CHILDREN: &[&str] = &["columns", "measures", "partitions", "hierarchies"];
const REPORT_CHILDREN: &[&str] = &["pages", "measures"];
const PAGE_CHILDREN: &[&str] = &["visuals"];

fn strip_local(value: &mut Value) {
    let Value::Object(map) = value else {
        return;
    };
    map.remove("id");
    map.remove("origin");
    for key in TYPED_ARRAYS {
        if let Some(Value::Array(items)) = map.get_mut(*key) {
            items.iter_mut().for_each(strip_local);
        }
    }
    for key in TYPED_OBJECTS {
        if let Some(child) = map.get_mut(*key) {
            strip_local(child);
        }
    }
}

/// The comparable properties of an object, without `exclude`.
fn properties<T: Serialize>(object: &T, exclude: &[&str]) -> Map<String, Value> {
    let mut value = serde_json::to_value(object).unwrap_or(Value::Null);
    strip_local(&mut value);
    match value {
        Value::Object(mut map) => {
            for key in exclude {
                map.remove(*key);
            }
            map
        }
        _ => Map::new(),
    }
}

fn changed_properties(
    before: &Map<String, Value>,
    after: &Map<String, Value>,
) -> Vec<PropertyDiff> {
    let mut out: Vec<PropertyDiff> = after
        .iter()
        .filter(|(k, v)| before.get(*k) != Some(*v))
        .map(|(k, v)| PropertyDiff {
            name: k.clone(),
            before: before.get(k).cloned(),
            after: Some(v.clone()),
        })
        .collect();
    out.extend(
        before
            .iter()
            .filter(|(k, _)| !after.contains_key(*k))
            .map(|(k, v)| PropertyDiff {
                name: k.clone(),
                before: Some(v.clone()),
                after: None,
            }),
    );
    out
}

fn all_properties(map: Map<String, Value>, added: bool) -> Vec<PropertyDiff> {
    map.into_iter()
        .map(|(name, v)| PropertyDiff {
            name,
            before: (!added).then(|| v.clone()),
            after: added.then_some(v),
        })
        .collect()
}

/// Pairs the items of two lists by key: those of `after` in order (with
/// their match in `before`, if any), then the unmatched ones of `before`.
/// Duplicate keys pair up in order.
fn pair_by<'a, T>(
    before: &'a [T],
    after: &'a [T],
    key: impl Fn(&T) -> String,
) -> Vec<(Option<&'a T>, Option<&'a T>)> {
    let mut index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, item) in before.iter().enumerate().rev() {
        index.entry(key(item)).or_default().push(i);
    }
    let mut used = vec![false; before.len()];
    let mut out = Vec::with_capacity(after.len());
    for item in after {
        let matched = index.get_mut(&key(item)).and_then(Vec::pop);
        if let Some(i) = matched
            && let Some(slot) = used.get_mut(i)
        {
            *slot = true;
        }
        out.push((matched.and_then(|i| before.get(i)), Some(item)));
    }
    out.extend(
        before
            .iter()
            .zip(&used)
            .filter(|(_, used)| !**used)
            .map(|(item, _)| (Some(item), None)),
    );
    out
}

/// Objects with a source origin.
trait Located {
    fn origin(&self) -> Option<&Origin>;
}

macro_rules! located {
    ($($t:ty),* $(,)?) => {
        $(impl Located for $t {
            fn origin(&self) -> Option<&Origin> {
                self.origin.as_ref()
            }
        })*
    };
}

located!(
    Model,
    Database,
    Table,
    Column,
    Measure,
    Partition,
    Hierarchy,
    CalculationItem,
    Relationship,
    NamedExpression,
    Role,
    Report,
    Page,
    Visual,
);

macro_rules! unlocated {
    ($($t:ty),* $(,)?) => {
        $(impl Located for $t {
            fn origin(&self) -> Option<&Origin> {
                None
            }
        })*
    };
}

unlocated!(QueryGroup, Culture, Perspective, ReportMeasure);

#[derive(Default)]
struct Differ {
    out: Vec<ObjectDiff>,
    /// Per entry of `out`, the [`TmdlIndex`] key of a model object.
    keys: Vec<Option<String>>,
}

fn is_report_kind(kind: DiffKind) -> bool {
    matches!(
        kind,
        DiffKind::Report | DiffKind::ReportMeasure | DiffKind::Page | DiffKind::Visual
    )
}

impl Differ {
    /// Records the difference of one object; returns whether it is in both
    /// documents (so its children are compared too). `key` finds a model
    /// object's TMDL text (unused for report objects, whose snippet is
    /// their JSON).
    fn object<T: Serialize + Located>(
        &mut self,
        kind: DiffKind,
        path: String,
        key: String,
        before: Option<&T>,
        after: Option<&T>,
        children: &[&str],
    ) -> bool {
        let report = is_report_kind(kind);
        let (change, props, snippet) = match (before, after) {
            (Some(b), Some(a)) => {
                let (bp, ap) = (properties(b, children), properties(a, children));
                let props = changed_properties(&bp, &ap);
                if props.is_empty() {
                    return true;
                }
                let snippet = report.then(|| (Some(json_snippet(bp)), Some(json_snippet(ap))));
                (DiffChange::Changed, props, snippet)
            }
            (None, Some(a)) => {
                let ap = properties(a, &[]);
                let snippet = report.then(|| (None, Some(json_snippet(ap.clone()))));
                (DiffChange::Added, all_properties(ap, true), snippet)
            }
            (Some(b), None) => {
                let bp = properties(b, &[]);
                let snippet = report.then(|| (Some(json_snippet(bp.clone())), None));
                (DiffChange::Removed, all_properties(bp, false), snippet)
            }
            (None, None) => return false,
        };
        self.out.push(ObjectDiff {
            kind,
            path,
            change,
            properties: props,
            before: before.and_then(Located::origin).cloned(),
            after: after.and_then(Located::origin).cloned(),
            snippet: snippet.map(|(before, after)| Snippet {
                language: SnippetLanguage::Json,
                before,
                after,
            }),
        });
        self.keys.push((!report).then_some(key));
        change == DiffChange::Changed
    }

    fn models(&mut self, before: Option<&Model>, after: Option<&Model>) {
        let path = after
            .or(before)
            .map_or_else(String::new, |m| m.name.clone());
        let model_key = tmdl_key(DiffKind::Model, "");
        if !self.object(
            DiffKind::Model,
            path,
            model_key,
            before,
            after,
            MODEL_CHILDREN,
        ) {
            return;
        }
        let (Some(b), Some(a)) = (before, after) else {
            return;
        };
        self.object(
            DiffKind::Database,
            "database".to_owned(),
            tmdl_key(DiffKind::Database, ""),
            b.database.as_ref(),
            a.database.as_ref(),
            &[],
        );
        for (bt, at) in pair_by(&b.tables, &a.tables, |t| name_key(&t.name)) {
            self.table(bt, at);
        }
        for (br, ar) in pair_by(&b.relationships, &a.relationships, |r| r.name.clone()) {
            let path = ar.or(br).map_or_else(String::new, |r| {
                format!(
                    "{}[{}] -> {}[{}]",
                    r.from.table, r.from.column, r.to.table, r.to.column
                )
            });
            let key = ar.or(br).map_or_else(String::new, |r| r.name.clone());
            self.object(
                DiffKind::Relationship,
                path,
                tmdl_key(DiffKind::Relationship, &key),
                br,
                ar,
                &[],
            );
        }
        self.named(DiffKind::Expression, &b.expressions, &a.expressions, |e| {
            e.name.as_str()
        });
        self.named(
            DiffKind::QueryGroup,
            &b.query_groups,
            &a.query_groups,
            |g| g.folder.as_str(),
        );
        self.named(DiffKind::Role, &b.roles, &a.roles, |r| r.name.as_str());
        self.named(DiffKind::Culture, &b.cultures, &a.cultures, |c| {
            c.name.as_str()
        });
        self.named(
            DiffKind::Perspective,
            &b.perspectives,
            &a.perspectives,
            |p| p.name.as_str(),
        );
    }

    /// Objects without children, matched and shown by name.
    fn named<T: Serialize + Located>(
        &mut self,
        kind: DiffKind,
        before: &[T],
        after: &[T],
        name: fn(&T) -> &str,
    ) {
        for (b, a) in pair_by(before, after, |x| name_key(name(x))) {
            let path = a.or(b).map(|x| name(x).to_owned()).unwrap_or_default();
            let key = tmdl_key(kind, &name_key(&path));
            self.object(kind, path, key, b, a, &[]);
        }
    }

    fn table(&mut self, before: Option<&Table>, after: Option<&Table>) {
        let table = after.or(before).map_or("", |t| t.name.as_str()).to_owned();
        let table_key = tmdl_key(DiffKind::Table, &name_key(&table));
        let compared = match (before, after) {
            (Some(b), Some(a)) => {
                let mut bp = properties(b, TABLE_CHILDREN);
                let mut ap = properties(a, TABLE_CHILDREN);
                for p in [&mut bp, &mut ap] {
                    if let Some(Value::Object(group)) = p.get_mut("calculationGroup") {
                        group.remove("calculationItems");
                    }
                }
                let props = changed_properties(&bp, &ap);
                if !props.is_empty() {
                    self.out.push(ObjectDiff {
                        kind: DiffKind::Table,
                        path: table.clone(),
                        change: DiffChange::Changed,
                        properties: props,
                        before: b.origin.clone(),
                        after: a.origin.clone(),
                        snippet: None,
                    });
                    self.keys.push(Some(table_key));
                }
                Some((b, a))
            }
            _ => {
                self.object(
                    DiffKind::Table,
                    table.clone(),
                    table_key,
                    before,
                    after,
                    &[],
                );
                None
            }
        };
        let Some((b, a)) = compared else {
            return;
        };
        let keyed = |kind: DiffKind, name: String| {
            let key = member_key(kind, &table, &name);
            (format!("{table}[{name}]"), key)
        };
        for (bc, ac) in pair_by(&b.columns, &a.columns, |c| name_key(&c.name)) {
            let name = ac.or(bc).map_or_else(String::new, |c| c.name.clone());
            let (path, key) = keyed(DiffKind::Column, name);
            self.object(DiffKind::Column, path, key, bc, ac, &[]);
        }
        for (bm, am) in pair_by(&b.measures, &a.measures, |m| name_key(&m.name)) {
            let name = am.or(bm).map_or_else(String::new, |m| m.name.clone());
            let (path, key) = keyed(DiffKind::Measure, name);
            self.object(DiffKind::Measure, path, key, bm, am, &[]);
        }
        for (bh, ah) in pair_by(&b.hierarchies, &a.hierarchies, |h| name_key(&h.name)) {
            let name = ah.or(bh).map_or_else(String::new, |h| h.name.clone());
            let (path, key) = keyed(DiffKind::Hierarchy, name);
            self.object(DiffKind::Hierarchy, path, key, bh, ah, &[]);
        }
        for (bp, ap) in pair_by(&b.partitions, &a.partitions, |p| name_key(&p.name)) {
            let name = ap.or(bp).map_or_else(String::new, |p| p.name.clone());
            let (path, key) = keyed(DiffKind::Partition, name);
            self.object(DiffKind::Partition, path, key, bp, ap, &[]);
        }
        let no_items: &[CalculationItem] = &[];
        let items = |t: &Table| {
            t.calculation_group
                .as_ref()
                .map_or(no_items, |g| g.calculation_items.as_slice())
                .to_vec()
        };
        let (bi, ai) = (items(b), items(a));
        for (bc, ac) in pair_by(&bi, &ai, |c| name_key(&c.name)) {
            let name = ac.or(bc).map_or_else(String::new, |c| c.name.clone());
            let (path, key) = keyed(DiffKind::CalculationItem, name);
            self.object(DiffKind::CalculationItem, path, key, bc, ac, &[]);
        }
    }

    fn report(&mut self, before: Option<&Report>, after: Option<&Report>) {
        let report = after.or(before).map_or("", |r| r.name.as_str()).to_owned();
        if !self.object(
            DiffKind::Report,
            report.clone(),
            String::new(),
            before,
            after,
            REPORT_CHILDREN,
        ) {
            return;
        }
        let (Some(b), Some(a)) = (before, after) else {
            return;
        };
        let measure_key =
            |m: &ReportMeasure| format!("{}\u{0}{}", name_key(&m.table), name_key(&m.name));
        for (bm, am) in pair_by(&b.measures, &a.measures, measure_key) {
            let path = am
                .or(bm)
                .map_or_else(String::new, |m| format!("{report}/{}[{}]", m.table, m.name));
            self.object(DiffKind::ReportMeasure, path, String::new(), bm, am, &[]);
        }
        for (bp, ap) in pair_by(&b.pages, &a.pages, |p| p.name.clone()) {
            let page = ap.or(bp).map_or_else(String::new, |p| {
                let shown = if p.display_name.is_empty() {
                    &p.name
                } else {
                    &p.display_name
                };
                format!("{report}/{shown}")
            });
            if !self.object(
                DiffKind::Page,
                page.clone(),
                String::new(),
                bp,
                ap,
                PAGE_CHILDREN,
            ) {
                continue;
            }
            let (Some(bp), Some(ap)) = (bp, ap) else {
                continue;
            };
            for (bv, av) in pair_by(&bp.visuals, &ap.visuals, |v| v.name.clone()) {
                let path = av
                    .or(bv)
                    .map_or_else(String::new, |v| format!("{page}/{}", v.name));
                self.object(DiffKind::Visual, path, String::new(), bv, av, &[]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;
    use pbi_model::DataType;

    fn doc(model: Model) -> Document {
        Document {
            model: Some(model),
            ..Document::default()
        }
    }

    fn sales() -> Table {
        let mut t = Table::new("Sales");
        t.columns.push(Column::data("Amount", DataType::Decimal));
        t.measures.push(Measure::new("Total", "SUM(Sales[Amount])"));
        t
    }

    fn model(tables: Vec<Table>) -> Model {
        Model {
            name: "Model".into(),
            tables,
            ..Model::default()
        }
    }

    #[test]
    fn ids_origins_order_and_name_case_do_not_count() {
        let mut a = sales();
        a.id = pbi_model::ObjectId(7);
        a.measures[0].id = pbi_model::ObjectId(8);
        a.measures[0].origin = Some(Origin::file("tables/Sales.tmdl"));
        let mut b = sales();
        b.name = "SALES".into();
        let before = doc(model(vec![a, Table::new("Other")]));
        let after = doc(model(vec![Table::new("Other"), b]));
        let diff = diff_documents(&before, &after);
        let paths: Vec<_> = diff.objects.iter().map(|o| o.path.as_str()).collect();
        // Only the table's name property differs (its case).
        assert_eq!(paths, ["SALES"]);
        assert_eq!(diff.objects[0].properties[0].name, "name");
    }

    #[test]
    fn changed_added_and_removed_objects_are_listed_once() {
        let before = doc(model(vec![sales(), Table::new("Gone")]));
        let mut s = sales();
        s.measures[0].expression = "SUM(Sales[Amount]) * 2".into();
        s.measures.push(Measure::new("Count", "COUNTROWS(Sales)"));
        let mut added = Table::new("New");
        added.measures.push(Measure::new("One", "1"));
        let after = doc(model(vec![s, added]));
        let diff = diff_documents(&before, &after);
        let got: Vec<_> = diff
            .objects
            .iter()
            .map(|o| (o.kind, o.path.as_str(), o.change))
            .collect();
        assert_eq!(
            got,
            [
                (DiffKind::Measure, "Sales[Total]", DiffChange::Changed),
                (DiffKind::Measure, "Sales[Count]", DiffChange::Added),
                (DiffKind::Table, "New", DiffChange::Added),
                (DiffKind::Table, "Gone", DiffChange::Removed),
            ]
        );
        let total = &diff.objects[0].properties;
        assert_eq!(total.len(), 1);
        assert_eq!(total[0].name, "expression");
        assert_eq!(total[0].before, Some(Value::from("SUM(Sales[Amount])")));
        assert_eq!(total[0].after, Some(Value::from("SUM(Sales[Amount]) * 2")));
        // An added table carries its children, without document-local ids.
        let new = &diff.objects[2].properties;
        let measures = new.iter().find(|p| p.name == "measures").unwrap();
        assert_eq!(measures.before, None);
        let one = &measures.after.as_ref().unwrap()[0];
        assert!(one.get("id").is_none());
        assert_eq!(one["expression"], "1");
        assert_eq!(diff.count(DiffChange::Added), 2);
        assert!(diff_documents(&before, &before).is_empty());
    }

    #[test]
    fn untyped_json_keeps_its_ids() {
        let mut a = sales();
        a.extra.insert(
            "custom".into(),
            pbi_model::PropValue::Json(serde_json::json!({"id": 1})),
        );
        let mut b = sales();
        b.extra.insert(
            "custom".into(),
            pbi_model::PropValue::Json(serde_json::json!({"id": 2})),
        );
        let diff = diff_documents(&doc(model(vec![a])), &doc(model(vec![b])));
        assert_eq!(diff.objects.len(), 1);
        assert_eq!(diff.objects[0].properties[0].name, "extra");
    }

    #[test]
    fn duplicate_keys_pair_in_order() {
        let before = ["a", "a", "b"];
        let after = ["a", "c", "a"];
        let pairs = pair_by(&before, &after, |s| (*s).to_owned());
        let got: Vec<_> = pairs
            .iter()
            .map(|(b, a)| (b.copied(), a.copied()))
            .collect();
        assert_eq!(
            got,
            [
                (Some("a"), Some("a")),
                (None, Some("c")),
                (Some("a"), Some("a")),
                (Some("b"), None),
            ]
        );
    }

    #[test]
    fn long_snippets_are_cut_with_a_count() {
        let mut big = Table::new("Big");
        for i in 0..150 {
            big.columns
                .push(Column::data(format!("C{i}"), DataType::Decimal));
        }
        let diff = diff_documents(&doc(model(vec![])), &doc(model(vec![big])));
        let snippet = diff.objects[0].snippet.as_ref().unwrap();
        assert_eq!(snippet.before, None);
        let text = snippet.after.as_ref().unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines.len(), MAX_SNIPPET_LINES + 1);
        assert_eq!(lines[0], "table Big");
        assert!(
            lines[MAX_SNIPPET_LINES].starts_with("… (") && text.ends_with(" more lines)"),
            "{}",
            lines[MAX_SNIPPET_LINES]
        );
    }
}
