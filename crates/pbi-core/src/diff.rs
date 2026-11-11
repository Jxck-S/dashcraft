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

use std::collections::HashMap;

use pbi_model::names::name_key;
use pbi_model::{
    CalculationItem, Column, Culture, Database, Document, Hierarchy, Measure, Model,
    NamedExpression, Origin, Page, Partition, Perspective, QueryGroup, Relationship, Report,
    ReportMeasure, Role, Table, Visual,
};
use serde::Serialize;
use serde_json::{Map, Value};

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
}

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
    DocumentDiff { objects: d.out }
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
}

impl Differ {
    /// Records the difference of one object; returns whether it is in both
    /// documents (so its children are compared too).
    fn object<T: Serialize + Located>(
        &mut self,
        kind: DiffKind,
        path: String,
        before: Option<&T>,
        after: Option<&T>,
        children: &[&str],
    ) -> bool {
        let (change, props) = match (before, after) {
            (Some(b), Some(a)) => {
                let props = changed_properties(&properties(b, children), &properties(a, children));
                if props.is_empty() {
                    return true;
                }
                (DiffChange::Changed, props)
            }
            (None, Some(a)) => (DiffChange::Added, all_properties(properties(a, &[]), true)),
            (Some(b), None) => (
                DiffChange::Removed,
                all_properties(properties(b, &[]), false),
            ),
            (None, None) => return false,
        };
        self.out.push(ObjectDiff {
            kind,
            path,
            change,
            properties: props,
            before: before.and_then(Located::origin).cloned(),
            after: after.and_then(Located::origin).cloned(),
        });
        change == DiffChange::Changed
    }

    fn models(&mut self, before: Option<&Model>, after: Option<&Model>) {
        let path = after
            .or(before)
            .map_or_else(String::new, |m| m.name.clone());
        if !self.object(DiffKind::Model, path, before, after, MODEL_CHILDREN) {
            return;
        }
        let (Some(b), Some(a)) = (before, after) else {
            return;
        };
        self.object(
            DiffKind::Database,
            "database".to_owned(),
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
            self.object(DiffKind::Relationship, path, br, ar, &[]);
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
            self.object(kind, path, b, a, &[]);
        }
    }

    fn table(&mut self, before: Option<&Table>, after: Option<&Table>) {
        let table = after.or(before).map_or("", |t| t.name.as_str()).to_owned();
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
                    });
                }
                Some((b, a))
            }
            _ => {
                self.object(DiffKind::Table, table.clone(), before, after, &[]);
                None
            }
        };
        let Some((b, a)) = compared else {
            return;
        };
        let member = |name: &str| format!("{table}[{name}]");
        for (bc, ac) in pair_by(&b.columns, &a.columns, |c| name_key(&c.name)) {
            let path = member(&ac.or(bc).map_or_else(String::new, |c| c.name.clone()));
            self.object(DiffKind::Column, path, bc, ac, &[]);
        }
        for (bm, am) in pair_by(&b.measures, &a.measures, |m| name_key(&m.name)) {
            let path = member(&am.or(bm).map_or_else(String::new, |m| m.name.clone()));
            self.object(DiffKind::Measure, path, bm, am, &[]);
        }
        for (bh, ah) in pair_by(&b.hierarchies, &a.hierarchies, |h| name_key(&h.name)) {
            let path = member(&ah.or(bh).map_or_else(String::new, |h| h.name.clone()));
            self.object(DiffKind::Hierarchy, path, bh, ah, &[]);
        }
        for (bp, ap) in pair_by(&b.partitions, &a.partitions, |p| name_key(&p.name)) {
            let path = member(&ap.or(bp).map_or_else(String::new, |p| p.name.clone()));
            self.object(DiffKind::Partition, path, bp, ap, &[]);
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
            let path = member(&ac.or(bc).map_or_else(String::new, |c| c.name.clone()));
            self.object(DiffKind::CalculationItem, path, bc, ac, &[]);
        }
    }

    fn report(&mut self, before: Option<&Report>, after: Option<&Report>) {
        let report = after.or(before).map_or("", |r| r.name.as_str()).to_owned();
        if !self.object(
            DiffKind::Report,
            report.clone(),
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
            self.object(DiffKind::ReportMeasure, path, bm, am, &[]);
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
            if !self.object(DiffKind::Page, page.clone(), bp, ap, PAGE_CHILDREN) {
                continue;
            }
            let (Some(bp), Some(ap)) = (bp, ap) else {
                continue;
            };
            for (bv, av) in pair_by(&bp.visuals, &ap.visuals, |v| v.name.clone()) {
                let path = av
                    .or(bv)
                    .map_or_else(String::new, |v| format!("{page}/{}", v.name));
                self.object(DiffKind::Visual, path, bv, av, &[]);
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
}
