//! Model → canonical TMDL.
//!
//! Layout and property order follow what Power BI Desktop writes (tabs,
//! `///` descriptions, child objects separated by blank lines, `ref` lines
//! for collection order). Typed properties are written first in canonical
//! order, then `extra` properties, then child objects, then annotations.
//! Everything the reader puts in `extra` is written back, so
//! `load(write(model)) == model` (ignoring origins).

use std::borrow::Cow;
use std::collections::HashSet;

use pbi_model::{
    Annotation, CalculationGroup, CalculationGroupExpression, CalculationItem, Cardinality, Column,
    ColumnKind, CrossFilter, Culture, Database, ExpressionKind, Hierarchy, Level, Measure, Model,
    NamedExpression, Partition, PartitionMode, PartitionSource, Perspective, PropValue,
    PropertyBag, QueryGroup, RawObject, Relationship, Role, SecurityFilteringBehavior, Table,
    TableKind, TablePermission,
};

use crate::cst::{MAX_DEPTH, TmdlFile, parse_file};
use crate::expr::{EncodedExpr, encode_expression, quote_name, quote_text};
use crate::lexer::{LineEnding, is_keyword};
use crate::load::{COLON_VALUE_KEY, fragment};

/// Writer options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WriteOptions {
    /// Line terminator; [`LineEnding::None`] is treated as LF.
    pub line_ending: LineEnding,
}

/// Writes `model` as canonical TMDL files (LF line endings). Paths are
/// relative to the `definition/` folder: `database.tmdl`, `model.tmdl`,
/// `expressions.tmdl`, `relationships.tmdl`, `dataSources.tmdl`,
/// `functions.tmdl`, `tables/<table>.tmdl`, `roles/<role>.tmdl`,
/// `cultures/<culture>.tmdl`, `perspectives/<perspective>.tmdl`.
/// `database` overrides `model.database`.
pub fn write_model(model: &Model, database: Option<&Database>) -> Vec<(String, String)> {
    write_model_with(model, database, &WriteOptions::default())
}

/// [`write_model`] with options.
pub fn write_model_with(
    model: &Model,
    database: Option<&Database>,
    options: &WriteOptions,
) -> Vec<(String, String)> {
    let nl = match options.line_ending {
        LineEnding::CrLf => "\r\n",
        _ => "\n",
    };
    let new = || Out::new(nl);
    let mut files = Vec::new();
    if let Some(db) = database.or(model.database.as_ref()) {
        let mut out = new();
        out.database(db);
        files.push(("database.tmdl".to_owned(), out.s));
    }
    let extras = ModelExtras::split(&model.extra);
    let mut out = new();
    out.model_file(model, &extras);
    files.push(("model.tmdl".to_owned(), out.s));
    if !model.expressions.is_empty() {
        let mut out = new();
        for (i, e) in model.expressions.iter().enumerate() {
            if i > 0 {
                out.blank();
            }
            out.named_expression(e);
        }
        files.push(("expressions.tmdl".to_owned(), out.s));
    }
    if !model.relationships.is_empty() {
        let mut out = new();
        for (i, r) in model.relationships.iter().enumerate() {
            if i > 0 {
                out.blank();
            }
            out.relationship(r);
        }
        files.push(("relationships.tmdl".to_owned(), out.s));
    }
    for (path, items) in [
        ("dataSources.tmdl", &extras.data_sources),
        ("functions.tmdl", &extras.functions),
    ] {
        if items.is_empty() {
            continue;
        }
        let mut out = new();
        for (i, (k, v)) in items.iter().enumerate() {
            if i > 0 {
                out.blank();
            }
            out.prop(0, k, v, 0);
        }
        files.push((path.to_owned(), out.s));
    }
    let mut names = FileNames::default();
    for t in &model.tables {
        let mut out = new();
        out.table(t);
        files.push((names.path("tables", &t.name), out.s));
    }
    for r in &model.roles {
        let mut out = new();
        out.role(r);
        files.push((names.path("roles", &r.name), out.s));
    }
    for c in &model.cultures {
        let mut out = new();
        out.culture(c);
        files.push((names.path("cultures", &c.name), out.s));
    }
    for p in &model.perspectives {
        let mut out = new();
        out.perspective(p);
        files.push((names.path("perspectives", &p.name), out.s));
    }
    files
}

/// Like [`write_model`], but files whose content is semantically unchanged
/// compared with `previous` (the CSTs from [`crate::LoadedModel::files`])
/// are emitted verbatim, so unedited files stay byte-identical (comments,
/// quoting and ordering included) and only edited files are re-serialized
/// canonically. Previous files are matched by their path relative to the
/// `definition/` folder (`\` and `/` are equivalent, case-insensitive).
/// Previous files with no counterpart (deleted objects) are not returned.
pub fn write_model_preserving(
    model: &Model,
    database: Option<&Database>,
    previous: &[TmdlFile],
) -> Vec<(String, String)> {
    write_model(model, database)
        .into_iter()
        .map(|(path, text)| {
            let prev = previous
                .iter()
                .find(|f| definition_relative(&f.path).eq_ignore_ascii_case(&path));
            if let Some(prev) = prev
                && let Ok(cst) = parse_file(&path, &text)
                && fragment(&cst) == fragment(prev)
            {
                return (path, prev.to_string());
            }
            (path, text)
        })
        .collect()
}

/// `a\b/definition/tables/X.tmdl` → `tables/X.tmdl`.
fn definition_relative(path: &str) -> String {
    let p = path.replace('\\', "/");
    if let Some(i) = p.rfind("/definition/") {
        return p
            .get(i + "/definition/".len()..)
            .unwrap_or_default()
            .to_owned();
    }
    p.strip_prefix("definition/").unwrap_or(&p).to_owned()
}

/// Unique, file-system-safe file names per folder.
#[derive(Default)]
struct FileNames {
    used: HashSet<String>,
}

impl FileNames {
    fn path(&mut self, folder: &str, name: &str) -> String {
        let stem = file_stem(name);
        let mut candidate = stem.clone();
        let mut n = 2;
        while !self
            .used
            .insert(format!("{folder}/{candidate}").to_lowercase())
        {
            candidate = format!("{stem}~{n}");
            n += 1;
        }
        format!("{folder}/{candidate}.tmdl")
    }
}

/// Object name → file stem: characters invalid in file names (and a
/// trailing dot or space) are percent-encoded.
fn file_stem(name: &str) -> String {
    let mut out = String::new();
    let count = name.chars().count();
    for (i, c) in name.chars().enumerate() {
        let last = i + 1 == count;
        if matches!(
            c,
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '%'
        ) || c.is_control()
            || (last && matches!(c, '.' | ' '))
        {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

/// A TMDL keyword for an `extra` key: valid keys pass through, `ref <type>`
/// keys are kept, anything else is mangled into an identifier.
fn tmdl_key(key: &str) -> Cow<'_, str> {
    if is_keyword(key) {
        return Cow::Borrowed(key);
    }
    if let Some(rest) = key.strip_prefix("ref ")
        && is_keyword(rest)
    {
        return Cow::Borrowed(key);
    }
    let mut out: String = key
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if !out
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
    {
        out.insert(0, '_');
    }
    Cow::Owned(out)
}

/// One `extra` entry flattened out of lists.
type Entry<'a> = (&'a str, &'a PropValue);

/// Whether an entry is written as a child object (after a blank line).
fn is_child_object(value: &PropValue) -> bool {
    match value {
        PropValue::Object(o) => o.name.is_some(),
        PropValue::List(items) => items.first().is_some_and(is_child_object),
        _ => false,
    }
}

/// Splits a bag into (scalar-like, child objects) keeping key order.
fn split_bag<'a>(bag: &'a PropertyBag, skip: &[&str]) -> (Vec<Entry<'a>>, Vec<Entry<'a>>) {
    let mut scalars = Vec::new();
    let mut objects = Vec::new();
    for (k, v) in bag {
        if skip.iter().any(|s| s == k) {
            continue;
        }
        if is_child_object(v) {
            objects.push((k.as_str(), v));
        } else {
            scalars.push((k.as_str(), v));
        }
    }
    (scalars, objects)
}

/// `model.extra` sorted into the places it is written to.
#[derive(Default)]
struct ModelExtras<'a> {
    /// Model properties (level 1 under `model`).
    props: Vec<Entry<'a>>,
    /// Other named top-level objects (level 0 in model.tmdl).
    objects: Vec<Entry<'a>>,
    data_sources: Vec<Entry<'a>>,
    functions: Vec<Entry<'a>>,
    refs: Vec<Entry<'a>>,
}

impl<'a> ModelExtras<'a> {
    fn split(bag: &'a PropertyBag) -> Self {
        let mut out = ModelExtras::default();
        for (k, v) in bag {
            let entry = (k.as_str(), v);
            if k.starts_with("ref ") {
                out.refs.push(entry);
            } else if !is_child_object(v) {
                out.props.push(entry);
            } else if k.eq_ignore_ascii_case("dataSource") {
                out.data_sources.push(entry);
            } else if k.eq_ignore_ascii_case("function") {
                out.functions.push(entry);
            } else {
                out.objects.push(entry);
            }
        }
        out
    }
}

/// Text builder.
struct Out {
    s: String,
    nl: &'static str,
}

impl Out {
    fn new(nl: &'static str) -> Self {
        Out {
            s: String::new(),
            nl,
        }
    }

    fn line(&mut self, level: usize, text: &str) {
        for _ in 0..level {
            self.s.push('\t');
        }
        self.s.push_str(text);
        self.s.push_str(self.nl);
    }

    fn blank(&mut self) {
        self.s.push_str(self.nl);
    }

    fn description(&mut self, level: usize, description: Option<&str>) {
        let Some(d) = description else { return };
        for l in d.split('\n') {
            let l = l.trim_end();
            if l.is_empty() {
                self.line(level, "///");
            } else {
                self.line(level, &format!("/// {l}"));
            }
        }
    }

    /// `head = expr` with single-line, indented-block or fenced form.
    fn expr(&mut self, level: usize, head: &str, expr: &str, empty_needs_block: bool) {
        let block_line = |out: &mut Out, l: &str| {
            if l.is_empty() {
                out.blank();
            } else {
                out.line(level + 2, l);
            }
        };
        match encode_expression(expr, empty_needs_block) {
            EncodedExpr::Inline(s) if s.is_empty() => self.line(level, &format!("{head} =")),
            EncodedExpr::Inline(s) => self.line(level, &format!("{head} = {s}")),
            EncodedExpr::Block(lines) => {
                self.line(level, &format!("{head} ="));
                for l in &lines {
                    block_line(self, l);
                }
            }
            EncodedExpr::Fenced(lines) => {
                self.line(level, &format!("{head} = ```"));
                for l in &lines {
                    block_line(self, l);
                }
                self.line(level + 2, "```");
            }
        }
    }

    fn text(&mut self, level: usize, key: &str, value: Option<&str>) {
        if let Some(v) = value {
            self.raw_text(level, key, &quote_text(v));
        }
    }

    fn raw_text(&mut self, level: usize, key: &str, raw: &str) {
        if raw.is_empty() {
            self.line(level, &format!("{key}:"));
        } else {
            self.line(level, &format!("{key}: {raw}"));
        }
    }

    fn name_ref(&mut self, level: usize, key: &str, value: Option<&str>) {
        if let Some(v) = value {
            self.raw_text(level, key, &quote_name(v));
        }
    }

    fn flag(&mut self, level: usize, key: &str, value: bool) {
        if value {
            self.line(level, key);
        }
    }

    /// Writes one `extra` entry (lists are written item by item).
    fn prop(&mut self, level: usize, key: &str, value: &PropValue, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let key = tmdl_key(key);
        let key = key.as_ref();
        match value {
            PropValue::Bool(true) => self.line(level, key),
            PropValue::Bool(false) => self.line(level, &format!("{key}: false")),
            PropValue::Text(t) => self.raw_text(level, key, t.replace('\n', " ").trim()),
            PropValue::Expression(e) => self.expr(level, key, e, false),
            PropValue::Json(v) => {
                if let Some(b) = v.as_bool() {
                    self.prop(level, key, &PropValue::Bool(b), depth + 1);
                } else if let Some(s) = v.as_str() {
                    self.raw_text(level, key, &quote_text(s));
                } else if v.is_number() || v.is_null() {
                    self.raw_text(level, key, &v.to_string());
                } else if let Some(map) = v.as_object() {
                    self.line(level, key);
                    for (k, child) in map {
                        self.prop(level + 1, k, &PropValue::Json(child.clone()), depth + 1);
                    }
                } else {
                    self.expr(level, key, &v.to_string(), false);
                }
            }
            PropValue::Object(o) => self.raw_object(level, key, o, depth),
            PropValue::List(items) => {
                for (i, item) in items.iter().enumerate() {
                    if i > 0 && is_child_object(item) {
                        self.blank();
                    }
                    self.prop(level, key, item, depth + 1);
                }
            }
        }
    }

    fn raw_object(&mut self, level: usize, key: &str, o: &RawObject, depth: usize) {
        self.description(level, o.description.as_deref());
        let head = match &o.name {
            Some(n) => format!("{key} {}", quote_name(n)),
            None => key.to_owned(),
        };
        let colon = match o.properties.get(COLON_VALUE_KEY) {
            Some(PropValue::Text(t)) => Some(t.as_str()),
            _ => None,
        };
        let has_children = o.properties.keys().any(|k| k != COLON_VALUE_KEY);
        match (&o.expression, colon) {
            (Some(e), _) => self.expr(level, &head, e, o.name.is_none() && has_children),
            (None, Some(raw)) if o.name.is_none() => {
                self.raw_text(level, &head, raw.replace('\n', " ").trim());
            }
            (None, Some(raw)) => self.expr(level, &head, raw, false),
            (None, None) => self.line(level, &head),
        }
        self.bag_in_order(level + 1, &o.properties, depth + 1);
    }

    /// Writes a bag in insertion order (used inside raw objects).
    fn bag_in_order(&mut self, level: usize, bag: &PropertyBag, depth: usize) {
        for (k, v) in bag {
            if k == COLON_VALUE_KEY {
                continue;
            }
            if is_child_object(v) {
                self.blank();
            }
            self.prop(level, k, v, depth);
        }
    }

    /// Scalar extras, child-object extras, annotations, in that order;
    /// `skip` lists keys written elsewhere.
    fn extras_scalars(&mut self, level: usize, bag: &PropertyBag, skip: &[&str]) {
        let (scalars, _) = split_bag(bag, skip);
        for (k, v) in scalars {
            self.prop(level, k, v, 0);
        }
    }

    fn extras_objects(&mut self, level: usize, bag: &PropertyBag, skip: &[&str]) {
        let (_, objects) = split_bag(bag, skip);
        for (k, v) in objects {
            self.blank();
            self.prop(level, k, v, 0);
        }
    }

    fn annotations(&mut self, level: usize, annotations: &[Annotation]) {
        for a in annotations {
            self.blank();
            self.expr(
                level,
                &format!("annotation {}", quote_name(&a.name)),
                &a.value,
                false,
            );
        }
    }

    fn database(&mut self, db: &Database) {
        self.description(0, db.description.as_deref());
        match &db.name {
            Some(n) => self.line(0, &format!("database {}", quote_name(n))),
            None => self.line(0, "database"),
        }
        if let Some(level) = db.compatibility_level {
            self.raw_text(1, "compatibilityLevel", &level.to_string());
        }
        self.text(1, "compatibilityMode", db.compatibility_mode.as_deref());
        self.extras_scalars(1, &db.extra, &[]);
        self.extras_objects(1, &db.extra, &[]);
        self.annotations(1, &db.annotations);
    }

    fn model_file(&mut self, m: &Model, extras: &ModelExtras<'_>) {
        self.description(0, m.description.as_deref());
        self.line(0, &format!("model {}", quote_name(&m.name)));
        self.text(1, "culture", m.culture.as_deref());
        self.text(
            1,
            "defaultPowerBIDataSourceVersion",
            m.default_power_bi_data_source_version.as_deref(),
        );
        self.flag(
            1,
            "discourageImplicitMeasures",
            m.discourage_implicit_measures,
        );
        self.text(1, "sourceQueryCulture", m.source_query_culture.as_deref());
        for (k, v) in &extras.props {
            self.prop(1, k, v, 0);
        }
        for g in &m.query_groups {
            self.blank();
            self.query_group(g);
        }
        for (k, v) in &extras.objects {
            self.blank();
            self.prop(0, k, v, 0);
        }
        for a in &m.annotations {
            self.blank();
            self.expr(
                0,
                &format!("annotation {}", quote_name(&a.name)),
                &a.value,
                false,
            );
        }
        let groups: [(&str, Vec<&str>); 4] = [
            ("table", m.tables.iter().map(|t| t.name.as_str()).collect()),
            ("role", m.roles.iter().map(|r| r.name.as_str()).collect()),
            (
                "cultureInfo",
                m.cultures.iter().map(|c| c.name.as_str()).collect(),
            ),
            (
                "perspective",
                m.perspectives.iter().map(|p| p.name.as_str()).collect(),
            ),
        ];
        for (kind, names) in groups {
            if names.len() < 2 {
                continue;
            }
            self.blank();
            for n in names {
                self.line(0, &format!("ref {kind} {}", quote_name(n)));
            }
        }
        if !extras.refs.is_empty() {
            self.blank();
            for (k, v) in &extras.refs {
                self.prop(0, k, v, 0);
            }
        }
    }

    fn query_group(&mut self, g: &QueryGroup) {
        self.description(0, g.description.as_deref());
        self.line(0, &format!("queryGroup {}", quote_name(&g.folder)));
        self.extras_scalars(1, &g.extra, &[]);
        self.extras_objects(1, &g.extra, &[]);
        self.annotations(1, &g.annotations);
    }

    fn named_expression(&mut self, e: &NamedExpression) {
        self.description(0, e.description.as_deref());
        let head = format!("expression {}", quote_name(&e.name));
        self.expr(0, &head, &e.full_expression(), false);
        if e.kind != ExpressionKind::M {
            self.raw_text(1, "kind", &quote_text(e.kind.as_str()));
        }
        self.text(1, "lineageTag", e.lineage_tag.as_deref());
        self.name_ref(1, "queryGroup", e.query_group.as_deref());
        self.extras_scalars(1, &e.extra, &[]);
        self.extras_objects(1, &e.extra, &[]);
        self.annotations(1, &e.annotations);
    }

    fn relationship(&mut self, r: &Relationship) {
        self.line(0, &format!("relationship {}", quote_name(&r.name)));
        if !r.is_active {
            self.line(1, "isActive: false");
        }
        if r.cross_filtering_behavior != CrossFilter::OneDirection {
            self.raw_text(
                1,
                "crossFilteringBehavior",
                &quote_text(r.cross_filtering_behavior.as_str()),
            );
        }
        if r.from_cardinality != Cardinality::Many {
            self.raw_text(
                1,
                "fromCardinality",
                &quote_text(r.from_cardinality.as_str()),
            );
        }
        if r.to_cardinality != Cardinality::One {
            self.raw_text(1, "toCardinality", &quote_text(r.to_cardinality.as_str()));
        }
        if r.security_filtering_behavior != SecurityFilteringBehavior::OneDirection {
            self.raw_text(
                1,
                "securityFilteringBehavior",
                &quote_text(r.security_filtering_behavior.as_str()),
            );
        }
        self.flag(
            1,
            "relyOnReferentialIntegrity",
            r.rely_on_referential_integrity,
        );
        self.extras_scalars(1, &r.extra, &[]);
        for (key, col) in [("fromColumn", &r.from), ("toColumn", &r.to)] {
            if !col.table.is_empty() || !col.column.is_empty() {
                let text = format!("{}.{}", quote_name(&col.table), quote_name(&col.column));
                self.raw_text(1, key, &text);
            }
        }
        self.extras_objects(1, &r.extra, &[]);
        self.annotations(1, &r.annotations);
    }

    fn table(&mut self, t: &Table) {
        self.description(0, t.description.as_deref());
        self.line(0, &format!("table {}", quote_name(&t.name)));
        self.flag(1, "isHidden", t.is_hidden);
        self.flag(1, "isPrivate", t.is_private);
        self.text(1, "lineageTag", t.lineage_tag.as_deref());
        self.text(1, "sourceLineageTag", t.source_lineage_tag.as_deref());
        self.text(1, "dataCategory", t.data_category.as_deref());
        self.extras_scalars(1, &t.extra, &["calculationGroup"]);
        if let Some(group) = &t.calculation_group {
            self.blank();
            self.calculation_group(group);
        }
        if let Some(group) = t.extra.get("calculationGroup") {
            self.blank();
            self.prop(1, "calculationGroup", group, 0);
        }
        let calculated = t.kind() == TableKind::CalculatedTable;
        for m in &t.measures {
            self.blank();
            self.measure(m);
        }
        for c in &t.columns {
            self.blank();
            self.column(c, calculated);
        }
        for h in &t.hierarchies {
            self.blank();
            self.hierarchy(h);
        }
        for p in &t.partitions {
            self.blank();
            self.partition(p);
        }
        self.extras_objects(1, &t.extra, &["calculationGroup"]);
        self.annotations(1, &t.annotations);
    }

    fn calculation_group(&mut self, g: &CalculationGroup) {
        self.description(1, g.description.as_deref());
        self.line(1, "calculationGroup");
        if g.precedence != 0 {
            self.raw_text(2, "precedence", &g.precedence.to_string());
        }
        for (key, selection) in [
            (
                "multipleOrEmptySelectionExpression",
                &g.multiple_or_empty_selection_expression,
            ),
            ("noSelectionExpression", &g.no_selection_expression),
        ] {
            if let Some(e) = selection {
                self.selection_expression(key, e);
            }
        }
        self.extras_scalars(2, &g.extra, &[]);
        for item in &g.calculation_items {
            self.blank();
            self.calculation_item(item);
        }
        self.extras_objects(2, &g.extra, &[]);
        self.annotations(2, &g.annotations);
    }

    fn selection_expression(&mut self, key: &str, e: &CalculationGroupExpression) {
        self.description(2, e.description.as_deref());
        let has_children = e.format_string_definition.is_some()
            || !e.extra.is_empty()
            || !e.annotations.is_empty();
        self.expr(2, key, &e.expression, has_children);
        if let Some(f) = &e.format_string_definition {
            self.expr(3, "formatStringDefinition", f, false);
        }
        self.extras_scalars(3, &e.extra, &[]);
        self.extras_objects(3, &e.extra, &[]);
        self.annotations(3, &e.annotations);
    }

    fn calculation_item(&mut self, i: &CalculationItem) {
        self.description(2, i.description.as_deref());
        let head = format!("calculationItem {}", quote_name(&i.name));
        self.expr(2, &head, &i.expression, false);
        if let Some(o) = i.ordinal {
            self.raw_text(3, "ordinal", &o.to_string());
        }
        if let Some(f) = &i.format_string_definition {
            self.expr(3, "formatStringDefinition", f, false);
        }
        self.extras_scalars(3, &i.extra, &[]);
        self.extras_objects(3, &i.extra, &[]);
        self.annotations(3, &i.annotations);
    }

    fn measure(&mut self, m: &Measure) {
        self.description(1, m.description.as_deref());
        let head = format!("measure {}", quote_name(&m.name));
        self.expr(1, &head, &m.expression, false);
        self.text(2, "formatString", m.format_string.as_deref());
        self.text(2, "displayFolder", m.display_folder.as_deref());
        self.flag(2, "isHidden", m.is_hidden);
        self.text(2, "lineageTag", m.lineage_tag.as_deref());
        self.text(2, "dataCategory", m.data_category.as_deref());
        let extra = tmdl_format_string_definition(&m.extra);
        self.extras_scalars(2, &extra, &[]);
        self.extras_objects(2, &extra, &[]);
        self.annotations(2, &m.annotations);
    }

    fn column(&mut self, c: &Column, in_calculated_table: bool) {
        self.description(1, c.description.as_deref());
        let head = format!("column {}", quote_name(&c.name));
        match &c.kind {
            ColumnKind::Calculated { expression } => self.expr(1, &head, expression, false),
            _ => self.line(1, &head),
        }
        if c.data_type != pbi_model::DataType::Automatic {
            self.raw_text(2, "dataType", &quote_text(c.data_type.as_str()));
        }
        self.flag(2, "isHidden", c.is_hidden);
        self.flag(2, "isKey", c.is_key);
        self.flag(2, "isNameInferred", c.is_name_inferred);
        self.flag(2, "isDataTypeInferred", c.is_data_type_inferred);
        self.text(2, "formatString", c.format_string.as_deref());
        self.text(2, "displayFolder", c.display_folder.as_deref());
        self.text(2, "lineageTag", c.lineage_tag.as_deref());
        self.text(2, "sourceLineageTag", c.source_lineage_tag.as_deref());
        self.text(2, "dataCategory", c.data_category.as_deref());
        if let Some(s) = &c.summarize_by {
            self.raw_text(2, "summarizeBy", &quote_text(s.as_str()));
        }
        match &c.kind {
            ColumnKind::Data { source_column }
            | ColumnKind::CalculatedTableColumn { source_column }
                if !source_column.is_empty() =>
            {
                self.text(2, "sourceColumn", Some(source_column));
            }
            _ => {}
        }
        self.name_ref(2, "sortByColumn", c.sort_by_column.as_deref());
        let explicit_type = match &c.kind {
            ColumnKind::RowNumber => Some("rowNumber"),
            ColumnKind::Data { .. } if in_calculated_table => Some("data"),
            ColumnKind::CalculatedTableColumn { .. } if !in_calculated_table => {
                Some("calculatedTableColumn")
            }
            _ => None,
        };
        if let Some(t) = explicit_type {
            self.raw_text(2, "type", t);
        }
        self.extras_scalars(2, &c.extra, &[]);
        self.extras_objects(2, &c.extra, &[]);
        self.annotations(2, &c.annotations);
    }

    fn hierarchy(&mut self, h: &Hierarchy) {
        self.description(1, h.description.as_deref());
        self.line(1, &format!("hierarchy {}", quote_name(&h.name)));
        self.flag(2, "isHidden", h.is_hidden);
        self.text(2, "displayFolder", h.display_folder.as_deref());
        self.text(2, "lineageTag", h.lineage_tag.as_deref());
        self.extras_scalars(2, &h.extra, &[]);
        for l in &h.levels {
            self.blank();
            self.level(l);
        }
        self.extras_objects(2, &h.extra, &[]);
        self.annotations(2, &h.annotations);
    }

    fn level(&mut self, l: &Level) {
        self.description(2, l.description.as_deref());
        self.line(2, &format!("level {}", quote_name(&l.name)));
        if let Some(o) = l.ordinal {
            self.raw_text(3, "ordinal", &o.to_string());
        }
        self.text(3, "lineageTag", l.lineage_tag.as_deref());
        if !l.column.is_empty() {
            self.name_ref(3, "column", Some(&l.column));
        }
        self.extras_scalars(3, &l.extra, &[]);
        self.extras_objects(3, &l.extra, &[]);
        self.annotations(3, &l.annotations);
    }

    fn partition(&mut self, p: &Partition) {
        self.description(1, p.description.as_deref());
        let head = format!("partition {}", quote_name(&p.name));
        let kind = p.source.kind_name();
        if kind.is_empty() {
            self.line(1, &head);
        } else {
            self.expr(1, &head, kind, false);
        }
        if p.mode != PartitionMode::Default {
            self.raw_text(2, "mode", &quote_text(p.mode.as_str()));
        }
        self.name_ref(2, "queryGroup", p.query_group.as_deref());
        let companion = match p.extra.get("source") {
            Some(PropValue::Object(o)) if o.name.is_none() && o.expression.is_none() => {
                Some(o.as_ref())
            }
            _ => None,
        };
        let skip: &[&str] = if companion.is_some() {
            &["source"]
        } else {
            &[]
        };
        self.partition_source(&p.source, companion);
        self.extras_scalars(2, &p.extra, skip);
        self.extras_objects(2, &p.extra, skip);
        self.annotations(2, &p.annotations);
    }

    fn partition_source(&mut self, source: &PartitionSource, companion: Option<&RawObject>) {
        let empty = PropertyBag::new();
        let extra_props = companion.map_or(&empty, |c| &c.properties);
        self.description(2, companion.and_then(|c| c.description.as_deref()));
        match source {
            PartitionSource::M { expression } | PartitionSource::Calculated { expression } => {
                self.expr(2, "source", expression, !extra_props.is_empty());
                self.bag_in_order(3, extra_props, 1);
            }
            PartitionSource::Entity {
                entity_name,
                expression_source,
                schema_name,
            } => {
                match companion.and_then(|c| c.expression.as_deref()) {
                    Some(e) => self.expr(2, "source", e, true),
                    None => self.line(2, "source"),
                }
                self.text(3, "entityName", Some(entity_name));
                self.name_ref(3, "expressionSource", expression_source.as_deref());
                self.text(3, "schemaName", schema_name.as_deref());
                self.bag_in_order(3, extra_props, 1);
            }
            PartitionSource::Query { query, data_source } => {
                self.line(2, "source");
                self.expr(3, "query", query, false);
                self.name_ref(3, "dataSource", data_source.as_deref());
                self.bag_in_order(3, extra_props, 1);
            }
            PartitionSource::Unknown { props, .. } => {
                self.bag_in_order(2, props, 1);
                self.bag_in_order(2, extra_props, 1);
            }
        }
    }

    fn role(&mut self, r: &Role) {
        self.description(0, r.description.as_deref());
        self.line(0, &format!("role {}", quote_name(&r.name)));
        self.text(1, "modelPermission", r.model_permission.as_deref());
        self.extras_scalars(1, &r.extra, &[]);
        for tp in &r.table_permissions {
            self.blank();
            self.table_permission(tp);
        }
        self.extras_objects(1, &r.extra, &[]);
        self.annotations(1, &r.annotations);
    }

    fn table_permission(&mut self, tp: &TablePermission) {
        let head = format!("tablePermission {}", quote_name(&tp.table));
        match &tp.filter_expression {
            Some(f) => self.expr(1, &head, f, false),
            None => self.line(1, &head),
        }
        self.extras_scalars(2, &tp.extra, &[]);
        self.extras_objects(2, &tp.extra, &[]);
        self.annotations(2, &tp.annotations);
    }

    fn culture(&mut self, c: &Culture) {
        self.line(0, &format!("cultureInfo {}", quote_name(&c.name)));
        let companion = match c.extra.get("linguisticMetadata") {
            Some(PropValue::Object(o)) if o.name.is_none() && o.expression.is_none() => {
                Some(o.as_ref())
            }
            _ => None,
        };
        let skip: &[&str] = if companion.is_some() && c.linguistic_metadata.is_some() {
            &["linguisticMetadata"]
        } else {
            &[]
        };
        // The reader takes the first `linguisticMetadata` as the typed one.
        if let Some(lm) = &c.linguistic_metadata {
            let empty = PropertyBag::new();
            let props = companion.map_or(&empty, |o| &o.properties);
            self.blank();
            self.description(1, companion.and_then(|o| o.description.as_deref()));
            self.expr(1, "linguisticMetadata", lm, !props.is_empty());
            self.bag_in_order(2, props, 1);
        }
        self.extras_scalars(1, &c.extra, skip);
        self.extras_objects(1, &c.extra, skip);
        self.annotations(1, &c.annotations);
    }

    fn perspective(&mut self, p: &Perspective) {
        self.description(0, p.description.as_deref());
        self.line(0, &format!("perspective {}", quote_name(&p.name)));
        self.extras_scalars(1, &p.extra, &[]);
        self.extras_objects(1, &p.extra, &[]);
        self.annotations(1, &p.annotations);
    }
}

/// TOM JSON (TMSL) keeps a measure's dynamic format string as
/// `{"expression": …}`; TMDL writes it as `formatStringDefinition = …`.
fn tmdl_format_string_definition(extra: &PropertyBag) -> Cow<'_, PropertyBag> {
    let Some(PropValue::Json(value)) = extra.get("formatStringDefinition") else {
        return Cow::Borrowed(extra);
    };
    let Some(map) = value.as_object().filter(|m| m.len() == 1) else {
        return Cow::Borrowed(extra);
    };
    let text = match map.get("expression") {
        Some(e) if e.is_string() => e.as_str().map(str::to_owned),
        Some(e) => e.as_array().and_then(|lines| {
            lines
                .iter()
                .map(|l| l.as_str())
                .collect::<Option<Vec<_>>>()
                .map(|l| l.join("\n"))
        }),
        None => None,
    };
    let Some(text) = text else {
        return Cow::Borrowed(extra);
    };
    let mut bag = extra.clone();
    bag.insert("formatStringDefinition".into(), PropValue::Expression(text));
    Cow::Owned(bag)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_stems_are_safe_and_unique() {
        assert_eq!(file_stem("Web Sessions"), "Web Sessions");
        assert_eq!(file_stem("a/b:c"), "a%2Fb%3Ac");
        assert_eq!(file_stem("x."), "x%2E");
        assert_eq!(file_stem(""), "_");
        let mut names = FileNames::default();
        assert_eq!(names.path("tables", "Sales"), "tables/Sales.tmdl");
        assert_eq!(names.path("tables", "SALES"), "tables/SALES~2.tmdl");
        assert_eq!(names.path("roles", "Sales"), "roles/Sales.tmdl");
    }

    #[test]
    fn keys_are_valid_keywords() {
        assert_eq!(tmdl_key("isAvailableInMdx"), "isAvailableInMdx");
        assert_eq!(tmdl_key("ref expression"), "ref expression");
        assert_eq!(tmdl_key("@odata.type"), "_odata_type");
        assert_eq!(tmdl_key("1x"), "_1x");
        assert_eq!(tmdl_key(""), "_");
    }

    #[test]
    fn definition_relative_paths() {
        assert_eq!(
            definition_relative("X.SemanticModel\\definition\\tables\\A.tmdl"),
            "tables/A.tmdl"
        );
        assert_eq!(definition_relative("definition/model.tmdl"), "model.tmdl");
        assert_eq!(definition_relative("model.tmdl"), "model.tmdl");
    }
}
