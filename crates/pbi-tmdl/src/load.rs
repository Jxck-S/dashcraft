//! CST → [`pbi_model::Model`] mapping.
//!
//! Every node is first decoded into an [`Item`] (unquoted name, decoded
//! value, description). Known object types and properties map to typed
//! fields; everything else is stored in the owner's `extra` bag (see
//! [`to_prop`]) and reported once per (owner, keyword) as an Info
//! diagnostic, so the writer can emit it again.

use std::collections::HashSet;

use indexmap::IndexMap;
use pbi_model::names::{name_key, names_eq, parse_tmdl_column_ref};
use pbi_model::semantic::split_m_meta;
use pbi_model::{
    Annotation, CalculationGroup, CalculationGroupExpression, CalculationItem, Cardinality, Column,
    ColumnKind, CrossFilter, Culture, DataType, Database, Diagnostic, ExpressionKind, Hierarchy,
    Level, Measure, Model, NamedExpression, Origin, Partition, PartitionMode, PartitionSource,
    Perspective, PropValue, PropertyBag, QueryGroup, RawObject, Relationship, Role,
    SecurityFilteringBehavior, SummarizeBy, Table, TableKind, TablePermission,
};

use crate::cst::{Node, TmdlFile, codes};
use crate::expr::{quote_text, unquote_name, unquote_text};
use crate::lexer::Operator;

/// Decoded value of a node.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    /// Bare keyword (`isHidden`, `table X`).
    None,
    /// `name: value`, raw text after the colon.
    Text(String),
    /// `… = value`, decoded expression.
    Expr(String),
}

/// A decoded node.
#[derive(Debug, Clone)]
struct Item {
    keyword: String,
    /// Lower-cased keyword for case-insensitive matching.
    kw: String,
    is_ref: bool,
    name: Option<String>,
    val: Val,
    description: Option<String>,
    line: u32,
    children: Vec<Item>,
}

impl Item {
    fn from_node(node: &Node) -> Item {
        let val = match node.operator() {
            None => Val::None,
            Some(Operator::Colon) => Val::Text(node.value_raw().unwrap_or_default().to_owned()),
            Some(Operator::Equals) => Val::Expr(node.expression().unwrap_or_default()),
        };
        let keyword = node.keyword().to_owned();
        Item {
            kw: keyword.to_ascii_lowercase(),
            keyword,
            is_ref: node.is_ref(),
            name: node.name(),
            val,
            description: node.description(),
            line: node.line_number(),
            children: node.children.iter().map(Item::from_node).collect(),
        }
    }

    /// A plain property: no name, children, description or `ref`.
    fn is_simple(&self) -> bool {
        self.children.is_empty()
            && self.name.is_none()
            && self.description.is_none()
            && !self.is_ref
    }

    fn is_text(&self) -> bool {
        matches!(self.val, Val::Text(_))
    }

    fn name_or_empty(&self) -> String {
        self.name.clone().unwrap_or_default()
    }

    /// Key under which the item is stored in an `extra` bag.
    fn key(&self) -> String {
        if self.is_ref {
            format!("ref {}", self.keyword)
        } else {
            self.keyword.clone()
        }
    }

    /// Value after `=` (or the raw text after `:`), `""` if none.
    fn expr_or_empty(&self) -> String {
        match &self.val {
            Val::Expr(e) | Val::Text(e) => e.clone(),
            Val::None => String::new(),
        }
    }
}

/// Key in [`RawObject::properties`] holding the raw value of a
/// `name: value` line that also has a description or child lines (the
/// colon form has no slot in [`RawObject`]; `:` is never a TMDL keyword).
pub const COLON_VALUE_KEY: &str = ":";

/// Converts an unmodeled item to a [`PropValue`]: plain flags, scalars and
/// expressions stay scalar; anything with a name, children or description
/// becomes a [`RawObject`] (see [`COLON_VALUE_KEY`]).
fn to_prop(it: &Item) -> PropValue {
    if it.is_simple() {
        return match &it.val {
            Val::None => PropValue::Bool(true),
            Val::Text(t) => PropValue::Text(t.clone()),
            Val::Expr(e) => PropValue::Expression(e.clone()),
        };
    }
    let mut obj = RawObject {
        name: it.name.clone(),
        expression: match &it.val {
            Val::Expr(e) => Some(e.clone()),
            _ => None,
        },
        description: it.description.clone(),
        properties: PropertyBag::new(),
    };
    if let Val::Text(t) = &it.val {
        obj.properties
            .insert(COLON_VALUE_KEY.to_owned(), PropValue::Text(t.clone()));
    }
    for c in &it.children {
        put_prop(&mut obj.properties, c.key(), to_prop(c));
    }
    PropValue::Object(Box::new(obj))
}

/// Inserts into a bag; repeated keys become a [`PropValue::List`].
pub(crate) fn put_prop(bag: &mut PropertyBag, key: String, value: PropValue) {
    match bag.get_mut(&key) {
        None => {
            bag.insert(key, value);
        }
        Some(PropValue::List(items)) => match value {
            PropValue::List(more) => items.extend(more),
            v => items.push(v),
        },
        Some(existing) => {
            let prev = std::mem::replace(existing, PropValue::Bool(false));
            let mut items = vec![prev];
            match value {
                PropValue::List(more) => items.extend(more),
                v => items.push(v),
            }
            *existing = PropValue::List(items);
        }
    }
}

/// A plain `name = expression` property.
fn is_simple_expr(it: &Item) -> bool {
    it.is_simple() && matches!(it.val, Val::Expr(_))
}

fn text_of(it: &Item) -> Option<String> {
    match &it.val {
        Val::Text(t) if it.is_simple() => Some(unquote_text(t)),
        _ => None,
    }
}

fn bool_of(it: &Item) -> Option<bool> {
    if !it.is_simple() {
        return None;
    }
    match &it.val {
        Val::None => Some(true),
        Val::Text(t) => {
            let v = unquote_text(t);
            if v.eq_ignore_ascii_case("true") {
                Some(true)
            } else if v.eq_ignore_ascii_case("false") {
                Some(false)
            } else {
                None
            }
        }
        Val::Expr(_) => None,
    }
}

/// A `ref <type> <name>` ordering entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RefEntry {
    /// Normalized type: `table`, `role`, `culture`, `perspective`.
    pub kind: String,
    pub name: String,
    pub origin: Origin,
}

/// Per-load state shared by the object mappers.
#[derive(Default)]
struct Ctx {
    path: String,
    diags: Vec<Diagnostic>,
    unmodeled: IndexMap<(String, String), (usize, Origin)>,
    /// Columns with an explicit `type:` (table key, column key).
    explicit_kind: HashSet<(String, String)>,
}

#[derive(Clone, Copy)]
enum ExplicitKind {
    Data,
    Calculated,
    CalculatedTableColumn,
    RowNumber,
}

impl Ctx {
    fn origin(&self, it: &Item) -> Origin {
        Origin::file(self.path.clone()).with_line(it.line)
    }

    /// Stores an unmodeled item in `bag` and records it for the summary.
    fn extra(&mut self, bag: &mut PropertyBag, owner: &str, it: &Item) {
        let origin = self.origin(it);
        let entry = self
            .unmodeled
            .entry((owner.to_owned(), it.key()))
            .or_insert((0, origin));
        entry.0 += 1;
        put_prop(bag, it.key(), to_prop(it));
    }

    /// Stores an item whose value could not be read.
    fn invalid(&mut self, bag: &mut PropertyBag, owner: &str, it: &Item) {
        self.diags.push(
            Diagnostic::warning(
                codes::INVALID_VALUE,
                format!(
                    "unexpected value or shape for {owner} property `{}`; kept as an untyped property",
                    it.keyword
                ),
            )
            .with_origin(self.origin(it)),
        );
        put_prop(bag, it.key(), to_prop(it));
    }

    fn set_text(
        &mut self,
        slot: &mut Option<String>,
        bag: &mut PropertyBag,
        owner: &str,
        it: &Item,
    ) {
        match text_of(it) {
            Some(v) => {
                if slot.is_some() {
                    self.diags.push(
                        Diagnostic::warning(
                            codes::DUPLICATE_PROPERTY,
                            format!(
                                "{owner} property `{}` is set more than once; the last value wins",
                                it.keyword
                            ),
                        )
                        .with_origin(self.origin(it)),
                    );
                }
                *slot = Some(v);
            }
            None => self.invalid(bag, owner, it),
        }
    }

    fn set_name_ref(
        &mut self,
        slot: &mut Option<String>,
        bag: &mut PropertyBag,
        owner: &str,
        it: &Item,
    ) {
        self.set_text(slot, bag, owner, it);
        if let Some(v) = slot.as_mut() {
            *v = unquote_name(v);
        }
    }

    fn set_bool(&mut self, slot: &mut bool, bag: &mut PropertyBag, owner: &str, it: &Item) {
        match bool_of(it) {
            Some(b) => *slot = b,
            None => self.invalid(bag, owner, it),
        }
    }

    fn set_parsed<T>(
        &mut self,
        slot: &mut T,
        parse: impl Fn(&str) -> T,
        bag: &mut PropertyBag,
        owner: &str,
        it: &Item,
    ) {
        match text_of(it) {
            Some(v) => *slot = parse(&v),
            None => self.invalid(bag, owner, it),
        }
    }

    fn set_int(&mut self, slot: &mut Option<i64>, bag: &mut PropertyBag, owner: &str, it: &Item) {
        match text_of(it).and_then(|v| v.parse::<i64>().ok()) {
            Some(v) => *slot = Some(v),
            None => self.invalid(bag, owner, it),
        }
    }

    /// `///` on an object type whose model has no description field.
    fn untyped_description(&mut self, bag: &mut PropertyBag, it: &Item) {
        if let Some(d) = &it.description {
            put_prop(bag, "description".into(), PropValue::Text(quote_text(d)));
        }
    }

    fn annotation(
        &mut self,
        list: &mut Vec<Annotation>,
        bag: &mut PropertyBag,
        owner: &str,
        it: &Item,
    ) {
        if it.is_text() || !it.children.is_empty() || it.description.is_some() {
            self.extra(bag, owner, it);
            return;
        }
        list.push(Annotation::new(it.name_or_empty(), it.expr_or_empty()));
    }

    fn database(&mut self, it: &Item) -> Database {
        let mut db = Database {
            name: it.name.clone(),
            description: it.description.clone(),
            origin: Some(self.origin(it)),
            ..Database::default()
        };
        for c in &it.children {
            let owner = "database";
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut db.extra, owner, c),
                "compatibilitylevel" => {
                    self.set_int(&mut db.compatibility_level, &mut db.extra, owner, c)
                }
                "compatibilitymode" => {
                    self.set_text(&mut db.compatibility_mode, &mut db.extra, owner, c)
                }
                "description" if db.description.is_none() && text_of(c).is_some() => {
                    db.description = text_of(c)
                }
                "annotation" => self.annotation(&mut db.annotations, &mut db.extra, owner, c),
                _ => self.extra(&mut db.extra, owner, c),
            }
        }
        db
    }

    fn table(&mut self, it: &Item) -> Table {
        let mut t = Table::new(it.name_or_empty());
        t.description = it.description.clone();
        t.origin = Some(self.origin(it));
        let owner = "table";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut t.extra, owner, c),
                "measure" if !c.is_text() => {
                    let m = self.measure(c);
                    t.measures.push(m);
                }
                "column" if !c.is_text() => {
                    let col = self.column(c, &t.name);
                    t.columns.push(col);
                }
                "partition" if !c.is_text() => {
                    let p = self.partition(c);
                    t.partitions.push(p);
                }
                "hierarchy" if !c.is_text() => {
                    let h = self.hierarchy(c);
                    t.hierarchies.push(h);
                }
                "calculationgroup"
                    if t.calculation_group.is_none() && c.name.is_none() && c.val == Val::None =>
                {
                    t.calculation_group = Some(Box::new(self.calculation_group(c)));
                }
                "annotation" => self.annotation(&mut t.annotations, &mut t.extra, owner, c),
                "lineagetag" => self.set_text(&mut t.lineage_tag, &mut t.extra, owner, c),
                "sourcelineagetag" => {
                    self.set_text(&mut t.source_lineage_tag, &mut t.extra, owner, c)
                }
                "ishidden" => self.set_bool(&mut t.is_hidden, &mut t.extra, owner, c),
                "isprivate" => self.set_bool(&mut t.is_private, &mut t.extra, owner, c),
                "datacategory" => self.set_text(&mut t.data_category, &mut t.extra, owner, c),
                "description" if t.description.is_none() && text_of(c).is_some() => {
                    t.description = text_of(c)
                }
                _ => self.extra(&mut t.extra, owner, c),
            }
        }
        t
    }

    fn calculation_group(&mut self, it: &Item) -> CalculationGroup {
        let mut g = CalculationGroup {
            description: it.description.clone(),
            ..CalculationGroup::default()
        };
        let mut precedence = None;
        let owner = "calculationGroup";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut g.extra, owner, c),
                "precedence" => self.set_int(&mut precedence, &mut g.extra, owner, c),
                "calculationitem" if !c.is_text() => {
                    let item = self.calculation_item(c);
                    g.calculation_items.push(item);
                }
                "multipleoremptyselectionexpression"
                    if g.multiple_or_empty_selection_expression.is_none()
                        && c.name.is_none()
                        && !c.is_text() =>
                {
                    g.multiple_or_empty_selection_expression = Some(self.selection_expression(c));
                }
                "noselectionexpression"
                    if g.no_selection_expression.is_none() && c.name.is_none() && !c.is_text() =>
                {
                    g.no_selection_expression = Some(self.selection_expression(c));
                }
                "description" if g.description.is_none() && text_of(c).is_some() => {
                    g.description = text_of(c)
                }
                "annotation" => self.annotation(&mut g.annotations, &mut g.extra, owner, c),
                _ => self.extra(&mut g.extra, owner, c),
            }
        }
        g.precedence = precedence.unwrap_or_default();
        g
    }

    fn calculation_item(&mut self, it: &Item) -> CalculationItem {
        let mut item = CalculationItem::new(it.name_or_empty(), it.expr_or_empty());
        item.description = it.description.clone();
        item.origin = Some(self.origin(it));
        let owner = "calculationItem";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut item.extra, owner, c),
                "ordinal" => self.set_int(&mut item.ordinal, &mut item.extra, owner, c),
                "formatstringdefinition"
                    if item.format_string_definition.is_none() && is_simple_expr(c) =>
                {
                    item.format_string_definition = Some(c.expr_or_empty());
                }
                "description" if item.description.is_none() && text_of(c).is_some() => {
                    item.description = text_of(c)
                }
                "annotation" => self.annotation(&mut item.annotations, &mut item.extra, owner, c),
                _ => self.extra(&mut item.extra, owner, c),
            }
        }
        item
    }

    fn selection_expression(&mut self, it: &Item) -> CalculationGroupExpression {
        let mut e = CalculationGroupExpression::new(it.expr_or_empty());
        e.description = it.description.clone();
        let owner = "selectionExpression";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut e.extra, owner, c),
                "formatstringdefinition"
                    if e.format_string_definition.is_none() && is_simple_expr(c) =>
                {
                    e.format_string_definition = Some(c.expr_or_empty());
                }
                "annotation" => self.annotation(&mut e.annotations, &mut e.extra, owner, c),
                _ => self.extra(&mut e.extra, owner, c),
            }
        }
        e
    }

    fn measure(&mut self, it: &Item) -> Measure {
        let mut m = Measure::new(it.name_or_empty(), it.expr_or_empty());
        m.description = it.description.clone();
        m.origin = Some(self.origin(it));
        let owner = "measure";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut m.extra, owner, c),
                "formatstring" => self.set_text(&mut m.format_string, &mut m.extra, owner, c),
                "displayfolder" => self.set_text(&mut m.display_folder, &mut m.extra, owner, c),
                "lineagetag" => self.set_text(&mut m.lineage_tag, &mut m.extra, owner, c),
                "datacategory" => self.set_text(&mut m.data_category, &mut m.extra, owner, c),
                "ishidden" => self.set_bool(&mut m.is_hidden, &mut m.extra, owner, c),
                "description" if m.description.is_none() && text_of(c).is_some() => {
                    m.description = text_of(c)
                }
                "annotation" => self.annotation(&mut m.annotations, &mut m.extra, owner, c),
                _ => self.extra(&mut m.extra, owner, c),
            }
        }
        m
    }

    fn column(&mut self, it: &Item, table: &str) -> Column {
        let mut col = Column {
            name: it.name_or_empty(),
            description: it.description.clone(),
            origin: Some(self.origin(it)),
            ..Column::default()
        };
        let expression = match &it.val {
            Val::Expr(e) => Some(e.clone()),
            _ => None,
        };
        let mut source_column: Option<String> = None;
        let mut explicit: Option<ExplicitKind> = None;
        let mut summarize: Option<SummarizeBy> = None;
        let owner = "column";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut col.extra, owner, c),
                "datatype" => self.set_parsed(
                    &mut col.data_type,
                    DataType::parse,
                    &mut col.extra,
                    owner,
                    c,
                ),
                "ishidden" => self.set_bool(&mut col.is_hidden, &mut col.extra, owner, c),
                "iskey" => self.set_bool(&mut col.is_key, &mut col.extra, owner, c),
                "isnameinferred" => {
                    self.set_bool(&mut col.is_name_inferred, &mut col.extra, owner, c)
                }
                "isdatatypeinferred" => {
                    self.set_bool(&mut col.is_data_type_inferred, &mut col.extra, owner, c)
                }
                "formatstring" => self.set_text(&mut col.format_string, &mut col.extra, owner, c),
                "displayfolder" => self.set_text(&mut col.display_folder, &mut col.extra, owner, c),
                "datacategory" => self.set_text(&mut col.data_category, &mut col.extra, owner, c),
                "lineagetag" => self.set_text(&mut col.lineage_tag, &mut col.extra, owner, c),
                "sourcelineagetag" => {
                    self.set_text(&mut col.source_lineage_tag, &mut col.extra, owner, c)
                }
                "summarizeby" => self.set_parsed(
                    &mut summarize,
                    |v| Some(SummarizeBy::parse(v)),
                    &mut col.extra,
                    owner,
                    c,
                ),
                "sortbycolumn" => {
                    self.set_name_ref(&mut col.sort_by_column, &mut col.extra, owner, c)
                }
                "sourcecolumn" if expression.is_none() => {
                    self.set_text(&mut source_column, &mut col.extra, owner, c)
                }
                "type" => {
                    let kind = text_of(c).and_then(|v| match v.to_ascii_lowercase().as_str() {
                        "data" => Some(ExplicitKind::Data),
                        "calculated" => Some(ExplicitKind::Calculated),
                        "calculatedtablecolumn" => Some(ExplicitKind::CalculatedTableColumn),
                        "rownumber" => Some(ExplicitKind::RowNumber),
                        _ => None,
                    });
                    match kind {
                        Some(k) => explicit = Some(k),
                        None => self.extra(&mut col.extra, owner, c),
                    }
                }
                "description" if col.description.is_none() && text_of(c).is_some() => {
                    col.description = text_of(c)
                }
                "annotation" => self.annotation(&mut col.annotations, &mut col.extra, owner, c),
                _ => self.extra(&mut col.extra, owner, c),
            }
        }
        col.summarize_by = summarize;
        let mut source_unused = source_column.is_some();
        col.kind = match (expression, explicit) {
            (Some(expression), _) => ColumnKind::Calculated { expression },
            (None, Some(ExplicitKind::RowNumber)) => ColumnKind::RowNumber,
            (None, Some(ExplicitKind::Calculated)) => ColumnKind::Calculated {
                expression: String::new(),
            },
            (None, Some(ExplicitKind::CalculatedTableColumn)) => {
                source_unused = false;
                ColumnKind::CalculatedTableColumn {
                    source_column: source_column.clone().unwrap_or_default(),
                }
            }
            (None, _) => {
                source_unused = false;
                ColumnKind::Data {
                    source_column: source_column.clone().unwrap_or_default(),
                }
            }
        };
        if source_unused && let Some(src) = source_column {
            put_prop(
                &mut col.extra,
                "sourceColumn".into(),
                PropValue::Text(quote_text(&src)),
            );
        }
        if explicit.is_some() {
            self.explicit_kind
                .insert((name_key(table), name_key(&col.name)));
        }
        col
    }

    fn partition(&mut self, it: &Item) -> Partition {
        let mut p = Partition {
            name: it.name_or_empty(),
            description: it.description.clone(),
            origin: Some(self.origin(it)),
            ..Partition::default()
        };
        let kind = it.expr_or_empty().trim().to_owned();
        let mut source: Option<&Item> = None;
        let owner = "partition";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut p.extra, owner, c),
                "mode" => {
                    self.set_parsed(&mut p.mode, PartitionMode::parse, &mut p.extra, owner, c)
                }
                "querygroup" => self.set_name_ref(&mut p.query_group, &mut p.extra, owner, c),
                "source" if source.is_none() => source = Some(c),
                "description" if p.description.is_none() && text_of(c).is_some() => {
                    p.description = text_of(c)
                }
                "annotation" => self.annotation(&mut p.annotations, &mut p.extra, owner, c),
                _ => self.extra(&mut p.extra, owner, c),
            }
        }
        p.source = self.partition_source(&kind, source, &mut p.extra);
        p
    }

    /// Builds the typed source; leftovers of the `source` node go to
    /// `extra["source"]` as a companion [`RawObject`] (no expression) that
    /// the writer merges back.
    fn partition_source(
        &mut self,
        kind: &str,
        source: Option<&Item>,
        extra: &mut PropertyBag,
    ) -> PartitionSource {
        let lower = kind.to_ascii_lowercase();
        let Some(src) = source else {
            return match lower.as_str() {
                "m" => PartitionSource::M {
                    expression: String::new(),
                },
                "calculated" => PartitionSource::Calculated {
                    expression: String::new(),
                },
                _ => PartitionSource::Unknown {
                    kind: kind.to_owned(),
                    props: PropertyBag::new(),
                },
            };
        };
        let mut companion = RawObject {
            description: src.description.clone(),
            ..RawObject::default()
        };
        let result = match lower.as_str() {
            "m" | "calculated" => {
                for c in &src.children {
                    put_prop(&mut companion.properties, c.key(), to_prop(c));
                }
                let expression = src.expr_or_empty();
                if lower == "m" {
                    PartitionSource::M { expression }
                } else {
                    PartitionSource::Calculated { expression }
                }
            }
            "entity" => {
                let mut entity_name = None;
                let mut expression_source = None;
                let mut schema_name = None;
                let owner = "partition source";
                for c in &src.children {
                    match c.kw.as_str() {
                        "entityname" if !c.is_ref => {
                            self.set_text(&mut entity_name, &mut companion.properties, owner, c)
                        }
                        "expressionsource" if !c.is_ref => self.set_name_ref(
                            &mut expression_source,
                            &mut companion.properties,
                            owner,
                            c,
                        ),
                        "schemaname" if !c.is_ref => {
                            self.set_text(&mut schema_name, &mut companion.properties, owner, c)
                        }
                        _ => put_prop(&mut companion.properties, c.key(), to_prop(c)),
                    }
                }
                companion.expression = match &src.val {
                    Val::None => None,
                    Val::Text(t) | Val::Expr(t) => Some(t.clone()),
                };
                PartitionSource::Entity {
                    entity_name: entity_name.unwrap_or_default(),
                    expression_source,
                    schema_name,
                }
            }
            "query" => {
                let mut query = match &src.val {
                    Val::None => None,
                    Val::Text(t) | Val::Expr(t) => Some(t.clone()),
                };
                let mut data_source = None;
                let owner = "partition source";
                for c in &src.children {
                    match c.kw.as_str() {
                        "query" if !c.is_ref && query.is_none() && c.is_simple() => {
                            query = Some(c.expr_or_empty())
                        }
                        "datasource" if !c.is_ref => {
                            self.set_name_ref(&mut data_source, &mut companion.properties, owner, c)
                        }
                        _ => put_prop(&mut companion.properties, c.key(), to_prop(c)),
                    }
                }
                PartitionSource::Query {
                    query: query.unwrap_or_default(),
                    data_source,
                }
            }
            _ => {
                let mut props = PropertyBag::new();
                put_prop(&mut props, src.key(), to_prop(src));
                return PartitionSource::Unknown {
                    kind: kind.to_owned(),
                    props,
                };
            }
        };
        if companion != RawObject::default() {
            put_prop(
                extra,
                "source".into(),
                PropValue::Object(Box::new(companion)),
            );
        }
        result
    }

    fn hierarchy(&mut self, it: &Item) -> Hierarchy {
        let mut h = Hierarchy {
            name: it.name_or_empty(),
            description: it.description.clone(),
            origin: Some(self.origin(it)),
            ..Hierarchy::default()
        };
        let owner = "hierarchy";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut h.extra, owner, c),
                "level" if !c.is_text() => {
                    let l = self.level(c);
                    h.levels.push(l);
                }
                "lineagetag" => self.set_text(&mut h.lineage_tag, &mut h.extra, owner, c),
                "ishidden" => self.set_bool(&mut h.is_hidden, &mut h.extra, owner, c),
                "displayfolder" => self.set_text(&mut h.display_folder, &mut h.extra, owner, c),
                "description" if h.description.is_none() && text_of(c).is_some() => {
                    h.description = text_of(c)
                }
                "annotation" => self.annotation(&mut h.annotations, &mut h.extra, owner, c),
                _ => self.extra(&mut h.extra, owner, c),
            }
        }
        h
    }

    fn level(&mut self, it: &Item) -> Level {
        let mut l = Level {
            name: it.name_or_empty(),
            description: it.description.clone(),
            ..Level::default()
        };
        let owner = "level";
        let mut column = None;
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut l.extra, owner, c),
                "ordinal" => self.set_int(&mut l.ordinal, &mut l.extra, owner, c),
                "column" if column.is_none() => {
                    self.set_name_ref(&mut column, &mut l.extra, owner, c)
                }
                "lineagetag" => self.set_text(&mut l.lineage_tag, &mut l.extra, owner, c),
                "description" if l.description.is_none() && text_of(c).is_some() => {
                    l.description = text_of(c)
                }
                "annotation" => self.annotation(&mut l.annotations, &mut l.extra, owner, c),
                _ => self.extra(&mut l.extra, owner, c),
            }
        }
        l.column = column.unwrap_or_default();
        l
    }

    fn relationship(&mut self, it: &Item) -> Relationship {
        let mut r = Relationship {
            name: it.name_or_empty(),
            origin: Some(self.origin(it)),
            ..Relationship::default()
        };
        self.untyped_description(&mut r.extra, it);
        let owner = "relationship";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut r.extra, owner, c),
                "fromcolumn" | "tocolumn" => {
                    match text_of(c).and_then(|v| parse_tmdl_column_ref(&v)) {
                        Some(col) if c.kw == "fromcolumn" => r.from = col,
                        Some(col) => r.to = col,
                        None => self.invalid(&mut r.extra, owner, c),
                    }
                }
                "fromcardinality" => self.set_parsed(
                    &mut r.from_cardinality,
                    Cardinality::parse,
                    &mut r.extra,
                    owner,
                    c,
                ),
                "tocardinality" => self.set_parsed(
                    &mut r.to_cardinality,
                    Cardinality::parse,
                    &mut r.extra,
                    owner,
                    c,
                ),
                "crossfilteringbehavior" => self.set_parsed(
                    &mut r.cross_filtering_behavior,
                    CrossFilter::parse,
                    &mut r.extra,
                    owner,
                    c,
                ),
                "securityfilteringbehavior" => self.set_parsed(
                    &mut r.security_filtering_behavior,
                    SecurityFilteringBehavior::parse,
                    &mut r.extra,
                    owner,
                    c,
                ),
                "isactive" => self.set_bool(&mut r.is_active, &mut r.extra, owner, c),
                "relyonreferentialintegrity" => {
                    self.set_bool(&mut r.rely_on_referential_integrity, &mut r.extra, owner, c)
                }
                "annotation" => self.annotation(&mut r.annotations, &mut r.extra, owner, c),
                _ => self.extra(&mut r.extra, owner, c),
            }
        }
        r
    }

    fn named_expression(&mut self, it: &Item) -> NamedExpression {
        let mut e = NamedExpression {
            name: it.name_or_empty(),
            description: it.description.clone(),
            origin: Some(self.origin(it)),
            ..NamedExpression::default()
        };
        let mut kind = ExpressionKind::M;
        let owner = "expression";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut e.extra, owner, c),
                "kind" => self.set_parsed(&mut kind, ExpressionKind::parse, &mut e.extra, owner, c),
                "lineagetag" => self.set_text(&mut e.lineage_tag, &mut e.extra, owner, c),
                "querygroup" => self.set_name_ref(&mut e.query_group, &mut e.extra, owner, c),
                "description" if e.description.is_none() && text_of(c).is_some() => {
                    e.description = text_of(c)
                }
                "annotation" => self.annotation(&mut e.annotations, &mut e.extra, owner, c),
                _ => self.extra(&mut e.extra, owner, c),
            }
        }
        let text = it.expr_or_empty();
        if kind == ExpressionKind::M {
            let (value, meta) = split_m_meta(&text);
            e.expression = value.to_owned();
            e.meta = meta.map(str::to_owned);
        } else {
            e.expression = text;
        }
        e.kind = kind;
        e
    }

    fn query_group(&mut self, it: &Item) -> QueryGroup {
        let mut g = QueryGroup {
            folder: it.name_or_empty(),
            description: it.description.clone(),
            ..QueryGroup::default()
        };
        let owner = "queryGroup";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut g.extra, owner, c),
                "description" if g.description.is_none() && text_of(c).is_some() => {
                    g.description = text_of(c)
                }
                "annotation" => self.annotation(&mut g.annotations, &mut g.extra, owner, c),
                _ => self.extra(&mut g.extra, owner, c),
            }
        }
        g
    }

    fn role(&mut self, it: &Item) -> Role {
        let mut r = Role {
            name: it.name_or_empty(),
            description: it.description.clone(),
            origin: Some(self.origin(it)),
            ..Role::default()
        };
        let owner = "role";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut r.extra, owner, c),
                "modelpermission" => self.set_text(&mut r.model_permission, &mut r.extra, owner, c),
                "tablepermission" if !c.is_text() => {
                    let tp = self.table_permission(c);
                    r.table_permissions.push(tp);
                }
                "description" if r.description.is_none() && text_of(c).is_some() => {
                    r.description = text_of(c)
                }
                "annotation" => self.annotation(&mut r.annotations, &mut r.extra, owner, c),
                _ => self.extra(&mut r.extra, owner, c),
            }
        }
        r
    }

    fn table_permission(&mut self, it: &Item) -> TablePermission {
        let mut tp = TablePermission {
            table: it.name_or_empty(),
            filter_expression: match &it.val {
                Val::Expr(e) => Some(e.clone()),
                _ => None,
            },
            ..TablePermission::default()
        };
        self.untyped_description(&mut tp.extra, it);
        let owner = "tablePermission";
        for c in &it.children {
            match c.kw.as_str() {
                "annotation" if !c.is_ref => {
                    self.annotation(&mut tp.annotations, &mut tp.extra, owner, c)
                }
                _ => self.extra(&mut tp.extra, owner, c),
            }
        }
        tp
    }

    fn culture(&mut self, it: &Item) -> Culture {
        let mut cu = Culture {
            name: it.name_or_empty(),
            ..Culture::default()
        };
        self.untyped_description(&mut cu.extra, it);
        let owner = "culture";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut cu.extra, owner, c),
                "linguisticmetadata"
                    if !c.is_text() && c.name.is_none() && cu.linguistic_metadata.is_none() =>
                {
                    cu.linguistic_metadata = Some(c.expr_or_empty());
                    let mut companion = RawObject {
                        description: c.description.clone(),
                        ..RawObject::default()
                    };
                    for p in &c.children {
                        put_prop(&mut companion.properties, p.key(), to_prop(p));
                    }
                    if companion != RawObject::default() {
                        put_prop(
                            &mut cu.extra,
                            "linguisticMetadata".into(),
                            PropValue::Object(Box::new(companion)),
                        );
                    }
                }
                "annotation" => self.annotation(&mut cu.annotations, &mut cu.extra, owner, c),
                _ => self.extra(&mut cu.extra, owner, c),
            }
        }
        cu
    }

    fn perspective(&mut self, it: &Item) -> Perspective {
        let mut p = Perspective {
            name: it.name_or_empty(),
            description: it.description.clone(),
            ..Perspective::default()
        };
        let owner = "perspective";
        for c in &it.children {
            match c.kw.as_str() {
                _ if c.is_ref => self.extra(&mut p.extra, owner, c),
                "description" if p.description.is_none() && text_of(c).is_some() => {
                    p.description = text_of(c)
                }
                "annotation" => self.annotation(&mut p.annotations, &mut p.extra, owner, c),
                _ => self.extra(&mut p.extra, owner, c),
            }
        }
        p
    }
}

/// Maps parsed files to a model.
#[derive(Default)]
pub(crate) struct Loader {
    ctx: Ctx,
    model: Model,
    database: Option<Database>,
    refs: Vec<RefEntry>,
}

/// Result of [`Loader::finish`].
pub(crate) struct Loaded {
    pub model: Model,
    pub database: Option<Database>,
    pub refs: Vec<RefEntry>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Loader {
    pub fn add_file(&mut self, file: &TmdlFile) {
        self.ctx.path = file.path.clone();
        self.ctx.diags.extend(file.diagnostics.iter().cloned());
        for node in &file.nodes {
            let it = Item::from_node(node);
            self.model_child(&it);
        }
    }

    /// Top-level items and children of `model` (model children need no
    /// indentation in TMDL, so both are handled alike).
    fn model_child(&mut self, it: &Item) {
        if it.is_ref {
            self.reference(it);
            return;
        }
        let obj = !it.is_text();
        let owner = "model";
        let m = &mut self.model;
        let ctx = &mut self.ctx;
        match it.kw.as_str() {
            "database" if obj => {
                let db = ctx.database(it);
                match &mut self.database {
                    Some(existing) => merge_database(existing, db),
                    None => self.database = Some(db),
                }
            }
            "model" if obj => self.model_decl(it),
            "table" if obj => {
                let t = ctx.table(it);
                add_table(m, t);
            }
            "relationship" if obj => {
                let r = ctx.relationship(it);
                m.relationships.push(r);
            }
            "expression" if obj => {
                let e = ctx.named_expression(it);
                m.expressions.push(e);
            }
            "querygroup" if obj => {
                let g = ctx.query_group(it);
                m.query_groups.push(g);
            }
            "role" if obj => {
                let r = ctx.role(it);
                m.roles.push(r);
            }
            "culture" | "cultureinfo" if obj => {
                let c = ctx.culture(it);
                m.cultures.push(c);
            }
            "perspective" if obj => {
                let p = ctx.perspective(it);
                m.perspectives.push(p);
            }
            "annotation" => ctx.annotation(&mut m.annotations, &mut m.extra, owner, it),
            "culture" => ctx.set_text(&mut m.culture, &mut m.extra, owner, it),
            "defaultpowerbidatasourceversion" => ctx.set_text(
                &mut m.default_power_bi_data_source_version,
                &mut m.extra,
                owner,
                it,
            ),
            "sourcequeryculture" => {
                ctx.set_text(&mut m.source_query_culture, &mut m.extra, owner, it)
            }
            "discourageimplicitmeasures" => {
                ctx.set_bool(&mut m.discourage_implicit_measures, &mut m.extra, owner, it)
            }
            "description" if m.description.is_none() && text_of(it).is_some() => {
                m.description = text_of(it)
            }
            _ => ctx.extra(&mut m.extra, owner, it),
        }
    }

    fn model_decl(&mut self, it: &Item) {
        if let Some(name) = &it.name {
            self.model.name = name.clone();
        }
        if it.description.is_some() {
            self.model.description = it.description.clone();
        }
        if self.model.origin.is_none() {
            self.model.origin = Some(self.ctx.origin(it));
        }
        for c in &it.children {
            self.model_child(c);
        }
    }

    fn reference(&mut self, it: &Item) {
        let kind = match it.kw.as_str() {
            "table" => "table",
            "role" => "role",
            "culture" | "cultureinfo" => "culture",
            "perspective" => "perspective",
            _ => "",
        };
        if kind == "table" && !it.children.is_empty() {
            // `ref table X` with children: partial declaration of table X.
            let t = self.ctx.table(it);
            add_table(&mut self.model, t);
            return;
        }
        if kind.is_empty() || !it.children.is_empty() || it.is_text() || it.name.is_none() {
            self.ctx.extra(&mut self.model.extra, "model", it);
            return;
        }
        if it.description.is_some() {
            self.ctx.diags.push(
                Diagnostic::warning(
                    codes::ORPHAN_DESCRIPTION,
                    "a `///` description above a `ref` line is ignored",
                )
                .with_origin(self.ctx.origin(it)),
            );
        }
        self.refs.push(RefEntry {
            kind: kind.to_owned(),
            name: it.name_or_empty(),
            origin: self.ctx.origin(it),
        });
    }

    /// Post-processing: calculated-table column kinds, `ref` ordering
    /// (when `apply_refs`), and the unmodeled-property summary.
    pub fn finish(mut self, apply_refs: bool) -> Loaded {
        let explicit = std::mem::take(&mut self.ctx.explicit_kind);
        for t in &mut self.model.tables {
            if t.kind() != TableKind::CalculatedTable {
                continue;
            }
            let tk = name_key(&t.name);
            for c in &mut t.columns {
                if explicit.contains(&(tk.clone(), name_key(&c.name))) {
                    continue;
                }
                if let ColumnKind::Data { source_column } = &mut c.kind {
                    c.kind = ColumnKind::CalculatedTableColumn {
                        source_column: std::mem::take(source_column),
                    };
                }
            }
        }
        if apply_refs {
            let diags = &mut self.ctx.diags;
            let refs = &self.refs;
            reorder(&mut self.model.tables, refs, "table", |t| &t.name, diags);
            reorder(&mut self.model.roles, refs, "role", |r| &r.name, diags);
            reorder(
                &mut self.model.cultures,
                refs,
                "culture",
                |c| &c.name,
                diags,
            );
            reorder(
                &mut self.model.perspectives,
                refs,
                "perspective",
                |p| &p.name,
                diags,
            );
        }
        for ((owner, key), (count, origin)) in std::mem::take(&mut self.ctx.unmodeled) {
            self.ctx.diags.push(
                Diagnostic::info(
                    codes::UNMODELED,
                    format!(
                        "`{key}` on {owner} is not modeled; kept as an untyped property ({count} occurrence(s))"
                    ),
                )
                .with_origin(origin),
            );
        }
        if self.database.is_some() {
            self.model.database = self.database.clone();
        }
        Loaded {
            model: self.model,
            database: self.database,
            refs: self.refs,
            diagnostics: self.ctx.diags,
        }
    }
}

fn add_table(model: &mut Model, table: Table) {
    match model
        .tables
        .iter_mut()
        .find(|t| names_eq(&t.name, &table.name))
    {
        Some(existing) => merge_table(existing, table),
        None => model.tables.push(table),
    }
}

/// Partial declarations: children are appended, set properties override.
fn merge_table(dst: &mut Table, src: Table) {
    dst.columns.extend(src.columns);
    dst.measures.extend(src.measures);
    dst.partitions.extend(src.partitions);
    dst.hierarchies.extend(src.hierarchies);
    match (&mut dst.calculation_group, src.calculation_group) {
        (Some(d), Some(s)) => {
            let s = *s;
            d.calculation_items.extend(s.calculation_items);
            d.annotations.extend(s.annotations);
            for (k, v) in s.extra {
                put_prop(&mut d.extra, k, v);
            }
            if s.precedence != 0 {
                d.precedence = s.precedence;
            }
            if s.description.is_some() {
                d.description = s.description;
            }
            if s.multiple_or_empty_selection_expression.is_some() {
                d.multiple_or_empty_selection_expression = s.multiple_or_empty_selection_expression;
            }
            if s.no_selection_expression.is_some() {
                d.no_selection_expression = s.no_selection_expression;
            }
        }
        (d @ None, s) => *d = s,
        (Some(_), None) => {}
    }
    dst.annotations.extend(src.annotations);
    for (k, v) in src.extra {
        put_prop(&mut dst.extra, k, v);
    }
    for (d, s) in [
        (&mut dst.description, src.description),
        (&mut dst.lineage_tag, src.lineage_tag),
        (&mut dst.source_lineage_tag, src.source_lineage_tag),
        (&mut dst.data_category, src.data_category),
    ] {
        if s.is_some() {
            *d = s;
        }
    }
    dst.is_hidden |= src.is_hidden;
    dst.is_private |= src.is_private;
}

fn merge_database(dst: &mut Database, src: Database) {
    if src.name.is_some() {
        dst.name = src.name;
    }
    if src.compatibility_level.is_some() {
        dst.compatibility_level = src.compatibility_level;
    }
    if src.compatibility_mode.is_some() {
        dst.compatibility_mode = src.compatibility_mode;
    }
    if src.description.is_some() {
        dst.description = src.description;
    }
    dst.annotations.extend(src.annotations);
    for (k, v) in src.extra {
        put_prop(&mut dst.extra, k, v);
    }
}

/// Orders `items` by the `ref` entries of `kind`; unreferenced items keep
/// their order after the referenced ones.
fn reorder<T>(
    items: &mut Vec<T>,
    refs: &[RefEntry],
    kind: &str,
    name_of: impl Fn(&T) -> &str,
    diags: &mut Vec<Diagnostic>,
) {
    let wanted: Vec<&RefEntry> = refs.iter().filter(|r| r.kind == kind).collect();
    if wanted.is_empty() {
        return;
    }
    let mut pool: Vec<Option<T>> = items.drain(..).map(Some).collect();
    let mut out = Vec::with_capacity(pool.len());
    for r in wanted {
        let found = pool
            .iter()
            .position(|x| x.as_ref().is_some_and(|x| names_eq(name_of(x), &r.name)));
        match found.and_then(|i| pool.get_mut(i)).and_then(Option::take) {
            Some(x) => out.push(x),
            None => diags.push(
                Diagnostic::warning(
                    codes::MISSING_REF,
                    format!(
                        "`ref {kind} {}` does not match a declared {kind}; ignored",
                        r.name
                    ),
                )
                .with_origin(r.origin.clone()),
            ),
        }
    }
    out.extend(pool.into_iter().flatten());
    *items = out;
}

/// Clears every `origin` in the model (for comparing models loaded from
/// different files or line positions).
pub fn clear_origins(model: &mut Model) {
    model.origin = None;
    if let Some(db) = &mut model.database {
        db.origin = None;
    }
    for t in &mut model.tables {
        t.origin = None;
        t.columns.iter_mut().for_each(|c| c.origin = None);
        t.measures.iter_mut().for_each(|m| m.origin = None);
        t.partitions.iter_mut().for_each(|p| p.origin = None);
        t.hierarchies.iter_mut().for_each(|h| h.origin = None);
        if let Some(g) = &mut t.calculation_group {
            g.calculation_items.iter_mut().for_each(|i| i.origin = None);
        }
    }
    model.relationships.iter_mut().for_each(|r| r.origin = None);
    model.expressions.iter_mut().for_each(|e| e.origin = None);
    model.roles.iter_mut().for_each(|r| r.origin = None);
}

/// Model fragment of a single file, used to decide whether a previous file
/// can be re-emitted verbatim.
#[derive(Debug, PartialEq)]
pub(crate) struct Fragment {
    model: Model,
    database: Option<Database>,
    refs: Vec<(String, String)>,
}

pub(crate) fn fragment(file: &TmdlFile) -> Fragment {
    let mut loader = Loader::default();
    loader.add_file(file);
    let loaded = loader.finish(false);
    let mut model = loaded.model;
    clear_origins(&mut model);
    let database = loaded.database.map(|mut d| {
        d.origin = None;
        d
    });
    Fragment {
        model,
        database,
        refs: loaded
            .refs
            .into_iter()
            .map(|r| (r.kind, name_key(&r.name)))
            .collect(),
    }
}
