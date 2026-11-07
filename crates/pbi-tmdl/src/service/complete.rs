//! Completion and hover for TMDL text, from the keyword catalog ([`super::catalog`]) and the
//! names the model's files declare.
//!
//! The text being edited and the other files of the folder are read tolerantly
//! ([`Outline`]): half-typed lines keep their structure, and the names declared anywhere in
//! the folder, the edited text included, are offered and described. What is offered where
//! follows the diagnostics: the keywords of the context the reader would read the line in, at
//! the database's compatibility level, without properties already set or properties after a
//! child object; the values of a property's type; the objects a reference may name.

use std::collections::{HashMap, HashSet};

use pbi_model::names::{name_key, parse_tmdl_column_ref, tmdl_name, tmdl_unquote};

use super::catalog::{Context, Form, Keyword, ValueType, catalog};
use super::diagnostics::{
    Member, OPAQUE, ROOT_REF_KINDS, child_context_name, context_label, ref_kind, truncate,
};
use super::outline::{Entry, Outline, Place};
use crate::lexer::{Operator, Span};

/// Most enumeration values a description lists.
const ENUM_LIST: usize = 24;

/// Another file of the model's folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileText<'a> {
    pub path: &'a str,
    pub text: &'a str,
}

/// What a suggestion or a hovered word is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    /// A keyword that starts an object (`column`, `measure`).
    Object,
    /// A property keyword (`dataType`, `isHidden`).
    Property,
    /// A value of an enumeration, or `true` / `false`.
    Value,
    /// The object type of a top-level `ref` line.
    ObjectType,
    Table,
    Column,
    Measure,
    Hierarchy,
    /// Another declared object: a query group, relationship, role, culture, perspective,
    /// expression, data source or function.
    Name,
}

/// One suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionItem {
    /// What is shown (`dataType`, `'Order Lines'.Quantity`).
    pub label: String,
    pub kind: ItemKind,
    /// The text that replaces the typed word: a keyword with `: `, ` ` or ` = ` after it on an
    /// otherwise empty line, a name quoted as TMDL writes it.
    pub insert_text: String,
    /// A keyword's forms (`dataType: <value>`), a value's enumeration, what a name is.
    pub detail: String,
    /// What a keyword takes and needs, or a declared object's `///` description.
    pub description: Option<String>,
}

/// The suggestions at a cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Completions {
    /// Byte range of the word a chosen item replaces (the cursor when nothing is typed yet).
    pub start: usize,
    pub end: usize,
    /// The typed text before the cursor that the items match.
    pub prefix: String,
    /// Best match first.
    pub items: Vec<CompletionItem>,
    /// More items matched than were returned.
    pub incomplete: bool,
}

/// Where a hovered object is declared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Index of the file in `others`; `None` for the text itself.
    pub file: Option<usize>,
    /// 1-based line of the declaration.
    pub line: u32,
}

/// What the word at a cursor is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// Byte range of the hovered word.
    pub start: usize,
    pub end: usize,
    pub kind: ItemKind,
    /// The keyword, the value, or the object's name (`'Order Lines'.Quantity`).
    pub name: String,
    /// The description as Markdown.
    pub markdown: String,
    /// Where the object a reference names is declared.
    pub declaration: Option<Declaration>,
}

/// The suggestions at byte `offset` of `text` (floored to a character boundary), best match
/// first, at most `max_items`. `others` are the other files of the model's folder: their
/// tables, columns, measures and other objects are offered where a reference names one, and
/// their `database` sets the compatibility level.
pub fn complete(
    text: &str,
    offset: usize,
    others: &[FileText<'_>],
    max_items: usize,
) -> Completions {
    let offset = floor_boundary(text, offset);
    let folder = Folder::new(text, others);
    let at_cursor = Span {
        start: offset,
        end: offset,
    };
    let (span, items) = match folder.text().place(offset) {
        Place::None | Place::Expression(_) => (at_cursor, Vec::new()),
        Place::Keyword {
            parent,
            level,
            line,
            own,
            span,
        } => {
            let rest_blank = rest_is_blank(text, span.end);
            let items = folder.keyword_items(parent, level, line, own, rest_blank);
            (span, items)
        }
        Place::Value { entry, span } => (span, folder.value_items(entry)),
        Place::Name { entry, span } => (span, folder.name_items(entry)),
        Place::RefKind { span } => (span, ref_kind_items(rest_is_blank(text, span.end))),
    };
    let typed_end = offset.clamp(span.start, span.end.max(span.start));
    let prefix = text
        .get(span.start..typed_end)
        .unwrap_or_default()
        .to_owned();
    let wanted = fold(&prefix);
    let mut ranked: Vec<(u8, String, CompletionItem)> = items
        .into_iter()
        .filter_map(|item| {
            let q = quality(&item.label, &wanted)?;
            Some((q, item.label.to_lowercase(), item))
        })
        .collect();
    ranked.sort_by(|a, b| {
        (a.0, &a.1, &a.2.label, &a.2.detail).cmp(&(b.0, &b.1, &b.2.label, &b.2.detail))
    });
    ranked.dedup_by(|a, b| a.2.label == b.2.label);
    let incomplete = ranked.len() > max_items;
    ranked.truncate(max_items);
    Completions {
        start: span.start,
        end: span.end,
        prefix,
        items: ranked.into_iter().map(|(_, _, item)| item).collect(),
        incomplete,
    }
}

/// What the word at byte `offset` of `text` is: a keyword (its forms, value and level), an
/// enumeration value, or a reference to a declared object (where it is declared and its `///`
/// description). `None` elsewhere.
pub fn hover(text: &str, offset: usize, others: &[FileText<'_>]) -> Option<Hover> {
    let offset = floor_boundary(text, offset);
    let folder = Folder::new(text, others);
    match folder.text().place(offset) {
        Place::Keyword {
            own: Some(e), span, ..
        } if span.end > span.start => folder.hover_keyword(e, span),
        Place::Value { entry, span } if span.end > span.start => folder.hover_value(entry, span),
        Place::Name { entry, span } if span.end > span.start => folder.hover_name(entry, span),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// The declarations of the folder

/// A structural line of one of the files: `source` 0 is the edited text, `i + 1` is
/// `others[i]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct At {
    source: usize,
    entry: usize,
}

/// Case-folded name → (name, where it is declared); the first declaration wins.
type Declared = HashMap<String, (String, At)>;

#[derive(Debug)]
struct TableDecl {
    name: String,
    at: At,
    columns: Declared,
    measures: Declared,
    hierarchies: Declared,
    /// Has a `partition … = calculated`.
    calculated: bool,
}

impl TableDecl {
    fn members(&self, member: Member) -> &Declared {
        match member {
            Member::Column => &self.columns,
            Member::Measure => &self.measures,
            Member::Hierarchy => &self.hierarchies,
        }
    }
}

/// What a top-level line declares children for.
enum Owner {
    Model,
    Database,
    Table(String),
}

/// What a reference may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// A column, measure or hierarchy of the table the line is in (a column also by path).
    Member(Member),
    /// `Table.Column` or `Table.Hierarchy`.
    Path(Member),
    /// A table or another top-level object, by the kind [`ref_kind`] indexes it under.
    Object(&'static str),
    /// A measure of any table.
    AnyMeasure,
}

struct Folder<'a> {
    text: &'a str,
    others: &'a [FileText<'a>],
    outline: Outline,
    /// The outlines of `others`.
    outlines: Vec<Outline>,
    tables: HashMap<String, TableDecl>,
    objects: HashMap<&'static str, Declared>,
    /// The `compatibilityLevel` of the first `database`.
    level: Option<u32>,
    /// Where the folder's `model` and `database` are declared: a model has one of each.
    singletons: Vec<(&'static str, At)>,
}

impl<'a> Folder<'a> {
    fn new(text: &'a str, others: &'a [FileText<'a>]) -> Self {
        let outline = Outline::new(text);
        let outlines: Vec<Outline> = others.iter().map(|f| Outline::new(f.text)).collect();
        let mut folder = Folder {
            text,
            others,
            outline: Outline::new(""),
            outlines: Vec::new(),
            tables: HashMap::new(),
            objects: HashMap::new(),
            level: None,
            singletons: Vec::new(),
        };
        folder.declare(0, &outline);
        for (i, other) in outlines.iter().enumerate() {
            folder.declare(i + 1, other);
        }
        folder.outline = outline;
        folder.outlines = outlines;
        folder
    }

    /// Records the tables (with their members), objects and compatibility level of a file, as
    /// the diagnostics index them.
    fn declare(&mut self, source: usize, outline: &Outline) {
        let mut owners: HashMap<usize, Owner> = HashMap::new();
        for (i, entry) in outline.entries.iter().enumerate() {
            if entry.is_ref {
                continue;
            }
            let at = At { source, entry: i };
            let keyword = outline.keyword(entry);
            let parent = match entry.parent {
                None if entry.level == 0 => None,
                None => continue,
                Some(p) => match (outline.entry(p), owners.get(&p)) {
                    (Some(pe), Some(owner)) if pe.level + 1 == entry.level => Some(owner),
                    _ => continue,
                },
            };
            match parent {
                Some(Owner::Table(key)) => {
                    let key = key.clone();
                    self.declare_member(outline, at, entry, &key);
                }
                Some(Owner::Database) => {
                    if self.level.is_none() && keyword.eq_ignore_ascii_case("compatibilityLevel") {
                        self.level = outline.value(entry).and_then(|v| v.trim().parse().ok());
                    }
                }
                Some(Owner::Model) | None => {
                    let top = parent.is_none();
                    if let Some(owner) = self.declare_object(outline, at, entry, top) {
                        owners.insert(i, owner);
                    }
                }
            }
        }
    }

    fn declare_object(
        &mut self,
        outline: &Outline,
        at: At,
        entry: &Entry,
        top: bool,
    ) -> Option<Owner> {
        let keyword = outline.keyword(entry);
        if keyword.eq_ignore_ascii_case("model") {
            self.singletons.extend(top.then_some(("model", at)));
            return top.then_some(Owner::Model);
        }
        if keyword.eq_ignore_ascii_case("database") {
            self.singletons.extend(top.then_some(("database", at)));
            return top.then_some(Owner::Database);
        }
        let name = outline.name(entry)?;
        if keyword.eq_ignore_ascii_case("table") {
            let key = name_key(&name);
            self.tables.entry(key.clone()).or_insert_with(|| TableDecl {
                name,
                at,
                columns: Declared::new(),
                measures: Declared::new(),
                hierarchies: Declared::new(),
                calculated: false,
            });
            return Some(Owner::Table(key));
        }
        if let Some(kind) = ref_kind(keyword) {
            self.objects
                .entry(kind)
                .or_default()
                .entry(name_key(&name))
                .or_insert((name, at));
        }
        None
    }

    fn declare_member(&mut self, outline: &Outline, at: At, entry: &Entry, table: &str) {
        let Some(t) = self.tables.get_mut(table) else {
            return;
        };
        let keyword = outline.keyword(entry);
        if keyword.eq_ignore_ascii_case("partition") {
            if entry.operator == Some(Operator::Equals)
                && outline
                    .value(entry)
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("calculated"))
            {
                t.calculated = true;
            }
            return;
        }
        let set = if keyword.eq_ignore_ascii_case("column") {
            &mut t.columns
        } else if keyword.eq_ignore_ascii_case("measure") {
            &mut t.measures
        } else if keyword.eq_ignore_ascii_case("hierarchy") {
            &mut t.hierarchies
        } else {
            return;
        };
        if let Some(name) = outline.name(entry) {
            set.entry(name_key(&name)).or_insert((name, at));
        }
    }

    /// The edited text's outline.
    fn text(&self) -> &Outline {
        &self.outline
    }

    fn outline_of(&self, source: usize) -> Option<&Outline> {
        match source.checked_sub(1) {
            None => Some(&self.outline),
            Some(i) => self.outlines.get(i),
        }
    }

    fn table(&self, name: &str) -> Option<&TableDecl> {
        self.tables.get(&name_key(name))
    }

    /// The context of the lines under `parent` in the edited text, and the table the names
    /// there belong to; `None` when the reader would not read them as an object's lines (under
    /// a property, an unknown keyword, a `ref` line, a culture's translations).
    fn context_at(&self, parent: Option<usize>) -> Option<(&'static Context, Option<String>)> {
        let outline = self.text();
        let catalog = catalog();
        let mut ctx = catalog.context("root")?;
        let mut table: Option<String> = None;
        for i in outline.path(parent) {
            let entry = outline.entry(i)?;
            if entry.is_ref {
                return None;
            }
            let word = outline.keyword(entry);
            if OPAQUE
                .iter()
                .any(|(c, k)| ctx.name == *c && word.eq_ignore_ascii_case(k))
            {
                return None;
            }
            let keyword = ctx.keyword(word)?;
            let form = Outline::form(entry);
            if !keyword.support(form).accepted() || !keyword.is_object(form) {
                return None;
            }
            if ["table", "tablePermission", "perspectiveTable"]
                .iter()
                .any(|k| k.eq_ignore_ascii_case(keyword.keyword))
                && let Some(name) = outline.name(entry)
            {
                table = Some(name);
            }
            let calculated = table
                .as_deref()
                .and_then(|t| self.table(t))
                .is_some_and(|t| t.calculated);
            let name = child_context_name(
                keyword.keyword,
                entry.operator,
                outline.value(entry),
                calculated,
            );
            ctx = catalog.context(name)?;
        }
        Some((ctx, table))
    }

    /// The forms of `keyword` accepted at the database's compatibility level.
    fn forms_at(&self, keyword: &Keyword) -> Vec<Form> {
        keyword
            .forms()
            .filter(|f| {
                self.level
                    .is_none_or(|level| keyword.support(*f).accepted_at(level))
            })
            .collect()
    }

    // -----------------------------------------------------------------------------------------
    // Completion

    fn keyword_items(
        &self,
        parent: Option<usize>,
        level: usize,
        line: usize,
        own: Option<usize>,
        rest_blank: bool,
    ) -> Vec<CompletionItem> {
        let Some((ctx, _)) = self.context_at(parent) else {
            return Vec::new();
        };
        let outline = self.text();
        let mut object_before = false;
        let mut property_after = false;
        let mut set: HashSet<String> = HashSet::new();
        for (i, sibling) in outline.children(parent, level) {
            if Some(i) == own || sibling.is_ref {
                continue;
            }
            let word = outline.keyword(sibling);
            let Some(keyword) = ctx.keyword(word) else {
                continue;
            };
            let form = Outline::form(sibling);
            if !keyword.support(form).accepted() {
                continue;
            }
            if keyword.is_object(form) {
                object_before |= sibling.line < line;
            } else {
                property_after |= sibling.line > line;
                set.insert(word.to_ascii_lowercase());
            }
        }
        let own_at = own.map(|entry| At { source: 0, entry });
        let declared_elsewhere = |keyword: &str| {
            ctx.name == "root"
                && self
                    .singletons
                    .iter()
                    .any(|(k, at)| *k == keyword && Some(*at) != own_at)
        };
        let mut items = Vec::new();
        for keyword in &ctx.keywords {
            if declared_elsewhere(keyword.keyword) {
                continue;
            }
            let forms: Vec<Form> = self
                .forms_at(keyword)
                .into_iter()
                .filter(|f| {
                    if keyword.is_object(*f) {
                        !property_after
                    } else {
                        !object_before && !set.contains(&keyword.keyword.to_ascii_lowercase())
                    }
                })
                .collect();
            if !forms.is_empty() {
                items.push(keyword_item(ctx, keyword, &forms, rest_blank));
            }
        }
        if ctx.name == "root" && !property_after {
            items.push(CompletionItem {
                label: "ref".to_owned(),
                kind: ItemKind::Object,
                insert_text: if rest_blank { "ref " } else { "ref" }.to_owned(),
                detail: "ref <type> <name>".to_owned(),
                description: Some(
                    "Names an object declared in another file, to set its order (`ref table \
                     Sales`)."
                        .to_owned(),
                ),
            });
        }
        if !property_after {
            for (_, keyword) in OPAQUE.iter().filter(|(c, _)| *c == ctx.name) {
                items.push(CompletionItem {
                    label: (*keyword).to_owned(),
                    kind: ItemKind::Object,
                    insert_text: (*keyword).to_owned(),
                    detail: (*keyword).to_owned(),
                    description: Some(format!(
                        "Object of {}: the translated names and captions of the model's \
                         objects.",
                        context_label(ctx)
                    )),
                });
            }
        }
        items
    }

    fn value_items(&self, e: usize) -> Vec<CompletionItem> {
        let outline = self.text();
        let Some(entry) = outline.entry(e).filter(|entry| !entry.is_ref) else {
            return Vec::new();
        };
        let Some((ctx, table)) = self.context_at(entry.parent) else {
            return Vec::new();
        };
        let word = outline.keyword(entry);
        let Some(keyword) = ctx.keyword(word) else {
            return Vec::new();
        };
        let value = |label: &str, detail: &str| CompletionItem {
            label: label.to_owned(),
            kind: ItemKind::Value,
            insert_text: label.to_owned(),
            detail: detail.to_owned(),
            description: None,
        };
        match keyword.value_type(Form::Colon) {
            ValueType::Bool => ["true", "false"]
                .iter()
                .map(|v| value(v, "boolean"))
                .collect(),
            ValueType::Enum(name) => catalog()
                .enum_values(name)
                .unwrap_or_default()
                .iter()
                .map(|v| value(v, name))
                .collect(),
            ValueType::Ref => value_target(ctx.name, word)
                .map(|t| self.candidates(t, table.as_deref()))
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    fn name_items(&self, e: usize) -> Vec<CompletionItem> {
        let outline = self.text();
        let Some(entry) = outline.entry(e) else {
            return Vec::new();
        };
        let word = outline.keyword(entry);
        if entry.is_ref {
            return match ref_kind(word) {
                Some(kind) if entry.level == 0 => self.candidates(Target::Object(kind), None),
                _ => Vec::new(),
            };
        }
        let Some((ctx, table)) = self.context_at(entry.parent) else {
            return Vec::new();
        };
        name_target(ctx.name, word)
            .map(|t| self.candidates(t, table.as_deref()))
            .unwrap_or_default()
    }

    /// The objects a reference may name.
    fn candidates(&self, target: Target, table: Option<&str>) -> Vec<CompletionItem> {
        let mut out = Vec::new();
        let mut push = |label: String, kind: ItemKind, detail: String, at: At| {
            out.push(CompletionItem {
                insert_text: label.clone(),
                label,
                kind,
                detail,
                description: self.description(at),
            });
        };
        match target {
            Target::Member(member) => {
                if let Some(t) = table.and_then(|t| self.table(t)) {
                    for (name, at) in t.members(member).values() {
                        push(
                            tmdl_name(name).into_owned(),
                            member_kind(member),
                            format!("{} of {}", member.label(), tmdl_name(&t.name)),
                            *at,
                        );
                    }
                }
            }
            Target::Path(member) => {
                for t in self.tables.values() {
                    for (name, at) in t.members(member).values() {
                        push(
                            format!("{}.{}", tmdl_name(&t.name), tmdl_name(name)),
                            member_kind(member),
                            member.label().to_owned(),
                            *at,
                        );
                    }
                }
            }
            Target::Object("table") => {
                for t in self.tables.values() {
                    push(
                        tmdl_name(&t.name).into_owned(),
                        ItemKind::Table,
                        "table".to_owned(),
                        t.at,
                    );
                }
            }
            Target::Object(kind) => {
                for (name, at) in self.objects.get(kind).into_iter().flat_map(|o| o.values()) {
                    push(
                        tmdl_name(name).into_owned(),
                        ItemKind::Name,
                        object_label(kind).to_owned(),
                        *at,
                    );
                }
            }
            Target::AnyMeasure => {
                for t in self.tables.values() {
                    for (name, at) in t.measures.values() {
                        push(
                            tmdl_name(name).into_owned(),
                            ItemKind::Measure,
                            format!("measure of {}", tmdl_name(&t.name)),
                            *at,
                        );
                    }
                }
            }
        }
        out
    }

    // -----------------------------------------------------------------------------------------
    // Hover

    fn hover_keyword(&self, e: usize, span: Span) -> Option<Hover> {
        let outline = self.text();
        let entry = outline.entry(e)?;
        let (ctx, _) = self.context_at(entry.parent)?;
        let keyword = ctx.keyword(outline.keyword(entry))?;
        let form = Outline::form(entry);
        let object = if keyword.support(form).accepted() {
            keyword.is_object(form)
        } else {
            keyword.forms().any(|f| keyword.is_object(f))
        };
        Some(Hover {
            start: span.start,
            end: span.end,
            kind: if object {
                ItemKind::Object
            } else {
                ItemKind::Property
            },
            name: keyword.keyword.to_owned(),
            markdown: keyword_markdown(ctx, keyword, self.level),
            declaration: None,
        })
    }

    fn hover_value(&self, e: usize, span: Span) -> Option<Hover> {
        let outline = self.text();
        let entry = outline.entry(e).filter(|entry| !entry.is_ref)?;
        let (ctx, table) = self.context_at(entry.parent)?;
        let word = outline.keyword(entry);
        let keyword = ctx.keyword(word)?;
        let value = self.text.get(span.start..span.end)?;
        match keyword.value_type(Form::Colon) {
            ValueType::Enum(name) => {
                let values = catalog().enum_values(name).unwrap_or_default();
                let mut markdown = format!("`{value}`: a value of the enumeration `{name}`");
                if !values.is_empty() {
                    markdown.push_str("\n\nValues: ");
                    markdown.push_str(&list(values));
                }
                Some(Hover {
                    start: span.start,
                    end: span.end,
                    kind: ItemKind::Value,
                    name: value.to_owned(),
                    markdown,
                    declaration: None,
                })
            }
            ValueType::Bool
                if value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("false") =>
            {
                Some(Hover {
                    start: span.start,
                    end: span.end,
                    kind: ItemKind::Value,
                    name: value.to_owned(),
                    markdown: format!("`{value}`: a boolean, `true` or `false`"),
                    declaration: None,
                })
            }
            ValueType::Ref => {
                let target = value_target(ctx.name, word)?;
                self.declared(target, table.as_deref(), value, span)
            }
            _ => None,
        }
    }

    fn hover_name(&self, e: usize, span: Span) -> Option<Hover> {
        let outline = self.text();
        let entry = outline.entry(e)?;
        let word = outline.keyword(entry);
        let name = self.text.get(span.start..span.end)?;
        if entry.is_ref {
            let kind = ref_kind(word).filter(|_| entry.level == 0)?;
            return self.declared(Target::Object(kind), None, name, span);
        }
        let (ctx, table) = self.context_at(entry.parent)?;
        let target = name_target(ctx.name, word)?;
        self.declared(target, table.as_deref(), name, span)
    }

    /// A hover for the object `raw` (as written) names.
    fn declared(
        &self,
        target: Target,
        table: Option<&str>,
        raw: &str,
        span: Span,
    ) -> Option<Hover> {
        let (kind, label, name, at) = self.resolve(target, table, raw)?;
        let outline = self.outline_of(at.source)?;
        let entry = outline.entry(at.entry)?;
        let line = outline.lines.get(entry.line).map_or(1, |l| l.number);
        let file = at.source.checked_sub(1);
        let mut markdown = format!("**{label}** `{name}`");
        if let Some(d) = outline.description(entry) {
            markdown.push_str("\n\n");
            markdown.push_str(&d);
        }
        let declared = outline.line_text(entry).trim();
        markdown.push_str(&format!("\n\n```\n{}\n```", truncate(declared, 200)));
        let place = match file.and_then(|i| self.others.get(i)) {
            Some(f) => format!("`{}`", f.path),
            None => "this file".to_owned(),
        };
        markdown.push_str(&format!("\n\nDeclared in {place}, line {line}."));
        Some(Hover {
            start: span.start,
            end: span.end,
            kind,
            name,
            markdown,
            declaration: Some(Declaration { file, line }),
        })
    }

    /// The object `raw` names: its kind, a label, its name as TMDL writes it, its declaration.
    fn resolve(
        &self,
        target: Target,
        table: Option<&str>,
        raw: &str,
    ) -> Option<(ItemKind, &'static str, String, At)> {
        let path = |member: Member| {
            let r = parse_tmdl_column_ref(raw)?;
            let t = self.table(&r.table)?;
            let (name, at) = t.members(member).get(&name_key(&r.column))?;
            Some((
                member_kind(member),
                member.label(),
                format!("{}.{}", tmdl_name(&t.name), tmdl_name(name)),
                *at,
            ))
        };
        match target {
            Target::Member(member) => {
                let local = table.and_then(|t| self.table(t)).and_then(|t| {
                    let (name, at) = t.members(member).get(&name_key(&tmdl_unquote(raw)))?;
                    Some((
                        member_kind(member),
                        member.label(),
                        format!("{}.{}", tmdl_name(&t.name), tmdl_name(name)),
                        *at,
                    ))
                });
                local.or_else(|| path(member))
            }
            Target::Path(member) => path(member),
            Target::Object("table") => {
                let t = self.table(&tmdl_unquote(raw))?;
                Some((
                    ItemKind::Table,
                    "table",
                    tmdl_name(&t.name).into_owned(),
                    t.at,
                ))
            }
            Target::Object(kind) => {
                let (name, at) = self.objects.get(kind)?.get(&name_key(&tmdl_unquote(raw)))?;
                Some((
                    ItemKind::Name,
                    object_label(kind),
                    tmdl_name(name).into_owned(),
                    *at,
                ))
            }
            Target::AnyMeasure => {
                let key = name_key(&tmdl_unquote(raw));
                let found = self.tables.values().find_map(|t| {
                    let (name, at) = t.measures.get(&key)?;
                    Some((
                        ItemKind::Measure,
                        "measure",
                        format!("{}.{}", tmdl_name(&t.name), tmdl_name(name)),
                        *at,
                    ))
                });
                found.or_else(|| path(Member::Measure))
            }
        }
    }

    /// The `///` description of a declaration.
    fn description(&self, at: At) -> Option<String> {
        let outline = self.outline_of(at.source)?;
        outline.description(outline.entry(at.entry)?)
    }
}

/// What a `keyword: value` reference names, as the diagnostics resolve it.
fn value_target(ctx: &str, keyword: &str) -> Option<Target> {
    let is = |c: &str, k: &str| ctx == c && keyword.eq_ignore_ascii_case(k);
    if matches!(ctx, "column" | "calculatedColumn" | "calculatedTableColumn")
        && keyword.eq_ignore_ascii_case("sortByColumn")
        || is("level", "column")
        || is("relatedColumnDetails", "groupByColumn")
    {
        Some(Target::Member(Member::Column))
    } else if is("relationship", "fromColumn")
        || is("relationship", "toColumn")
        || is("expression", "parameterValuesColumn")
        || is("alternateOf", "baseColumn")
        || is("variation", "defaultColumn")
    {
        Some(Target::Path(Member::Column))
    } else if is("variation", "defaultHierarchy") {
        Some(Target::Path(Member::Hierarchy))
    } else if keyword.eq_ignore_ascii_case("queryGroup")
        && matches!(
            ctx,
            "expression" | "partition" | "calculatedPartition" | "calculationGroupPartition"
        )
    {
        Some(Target::Object("queryGroup"))
    } else if is("variation", "relationship") {
        Some(Target::Object("relationship"))
    } else if is("alternateOf", "baseTable") {
        Some(Target::Object("table"))
    } else if is("model", "defaultMeasure") {
        Some(Target::AnyMeasure)
    } else {
        None
    }
}

/// What the name of `keyword <name>` refers to (`tablePermission Sales`), when it refers to an
/// object instead of declaring one.
fn name_target(ctx: &str, keyword: &str) -> Option<Target> {
    let is = |c: &str, k: &str| ctx == c && keyword.eq_ignore_ascii_case(k);
    if is("role", "tablePermission") || is("perspective", "perspectiveTable") {
        Some(Target::Object("table"))
    } else if is("tablePermission", "columnPermission")
        || is("perspectiveTable", "perspectiveColumn")
    {
        Some(Target::Member(Member::Column))
    } else if is("perspectiveTable", "perspectiveMeasure") {
        Some(Target::Member(Member::Measure))
    } else if is("perspectiveTable", "perspectiveHierarchy") {
        Some(Target::Member(Member::Hierarchy))
    } else {
        None
    }
}

/// The object types of a top-level `ref` line (`cultureInfo`, as Desktop writes it, for
/// cultures).
fn ref_kind_items(rest_blank: bool) -> Vec<CompletionItem> {
    ROOT_REF_KINDS
        .iter()
        .map(|(k, kind)| CompletionItem {
            label: (*k).to_owned(),
            kind: ItemKind::ObjectType,
            insert_text: if rest_blank {
                format!("{k} ")
            } else {
                (*k).to_owned()
            },
            detail: format!("ref {k} <name>"),
            description: Some(format!(
                "Refers to a {} declared in another file.",
                object_label(kind)
            )),
        })
        .collect()
}

fn keyword_item(
    ctx: &Context,
    keyword: &Keyword,
    forms: &[Form],
    rest_blank: bool,
) -> CompletionItem {
    let has = |f: Form| forms.contains(&f);
    let object = forms.iter().any(|f| keyword.is_object(*f));
    let property = forms.iter().any(|f| !keyword.is_object(*f));
    let colon = keyword.value_type(Form::Colon);
    let suffix = if !rest_blank {
        ""
    } else if has(Form::Colon)
        && colon != ValueType::None
        && !(colon == ValueType::Bool && has(Form::Bare))
    {
        ": "
    } else if has(Form::Named) || has(Form::NamedEquals) {
        " "
    } else if has(Form::Equals) && !has(Form::Bare) {
        " = "
    } else {
        ""
    };
    let detail: Vec<String> = forms.iter().map(|f| f.pattern(keyword.keyword)).collect();
    CompletionItem {
        label: keyword.keyword.to_owned(),
        kind: if object && !property {
            ItemKind::Object
        } else {
            ItemKind::Property
        },
        insert_text: format!("{}{suffix}", keyword.keyword),
        detail: detail.join(", "),
        description: Some(keyword_summary(ctx, keyword, forms)),
    }
}

/// `Property of `column`. Takes a DataType value: …. Needs compatibility level 1600 or higher.`
fn keyword_summary(ctx: &Context, keyword: &Keyword, forms: &[Form]) -> String {
    let object = forms.iter().any(|f| keyword.is_object(*f));
    let property = forms.iter().any(|f| !keyword.is_object(*f));
    let what = match (object, property) {
        (true, true) => "Object or property",
        (true, false) => "Object",
        _ => "Property",
    };
    let mut out = format!("{what} of {}.", context_label(ctx));
    let ty = forms
        .iter()
        .map(|f| keyword.value_type(*f))
        .find(|t| *t != ValueType::None);
    if let Some(ty) = ty {
        out.push_str(&format!(" Takes {}.", value_text(ty)));
    }
    if let Some(level) = keyword.min_level() {
        out.push_str(&format!(" Needs compatibility level {level} or higher."));
    }
    out
}

fn keyword_markdown(ctx: &Context, keyword: &Keyword, level: Option<u32>) -> String {
    let forms: Vec<Form> = keyword.forms().collect();
    let object = forms.iter().any(|f| keyword.is_object(*f));
    let property = forms.iter().any(|f| !keyword.is_object(*f));
    let what = match (object, property) {
        (true, true) => "object or property",
        (true, false) => "object",
        _ => "property",
    };
    let mut out = format!(
        "**`{}`**: {what} of {}",
        keyword.keyword,
        context_label(ctx)
    );
    let written: Vec<String> = forms
        .iter()
        .map(|f| format!("`{}`", f.pattern(keyword.keyword)))
        .collect();
    if !written.is_empty() {
        out.push_str(&format!("\n\nWritten {}", written.join(", ")));
    }
    for form in &forms {
        let ty = keyword.value_type(*form);
        if ty != ValueType::None {
            out.push_str(&format!(
                "\n\n`{}` takes {}",
                form.pattern(keyword.keyword),
                value_text(ty)
            ));
        }
    }
    if let Some(min) = keyword.min_level() {
        out.push_str(&format!("\n\nNeeds compatibility level {min} or higher"));
        if let Some(level) = level {
            out.push_str(&format!(" (the database is at {level})"));
        }
    }
    out
}

/// `a DataType value: automatic, string, …` or the type's description.
fn value_text(ty: ValueType) -> String {
    match ty {
        ValueType::Enum(name) => match catalog().enum_values(name) {
            Some(values) if !values.is_empty() => format!("a {name} value: {}", list(values)),
            _ => ty.describe(),
        },
        _ => ty.describe(),
    }
}

/// `` `a`, `b`, … `` (at most [`ENUM_LIST`]).
fn list(values: &[&str]) -> String {
    let mut shown: Vec<String> = values
        .iter()
        .take(ENUM_LIST)
        .map(|v| format!("`{v}`"))
        .collect();
    if values.len() > ENUM_LIST {
        shown.push("…".to_owned());
    }
    shown.join(", ")
}

fn member_kind(member: Member) -> ItemKind {
    match member {
        Member::Column => ItemKind::Column,
        Member::Measure => ItemKind::Measure,
        Member::Hierarchy => ItemKind::Hierarchy,
    }
}

fn object_label(kind: &str) -> &'static str {
    match kind {
        "table" => "table",
        "role" => "role",
        "culture" => "culture",
        "perspective" => "perspective",
        "expression" => "expression",
        "queryGroup" => "query group",
        "dataSource" => "data source",
        "relationship" => "relationship",
        "function" => "function",
        _ => "object",
    }
}

/// Whether the rest of the line after byte `at` is blank.
fn rest_is_blank(text: &str, at: usize) -> bool {
    text.get(at..)
        .and_then(|rest| rest.split(['\r', '\n']).next())
        .is_none_or(|rest| rest.trim().is_empty())
}

fn floor_boundary(text: &str, offset: usize) -> usize {
    let mut i = offset.min(text.len());
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Lower-cased, without quotes.
fn fold(s: &str) -> String {
    s.chars()
        .filter(|c| *c != '\'')
        .flat_map(char::to_lowercase)
        .collect()
}

/// How well `candidate` matches the folded `prefix`: 0 the same, 1 starts with it, 2 a word of
/// it does (after a non-alphanumeric character or at a lower-to-upper case change), 3 contains
/// it; `None` when it does not match. Quotes are ignored.
fn quality(candidate: &str, prefix: &str) -> Option<u8> {
    if prefix.is_empty() {
        return Some(1);
    }
    let candidate: String = candidate.chars().filter(|c| *c != '\'').collect();
    let lower = candidate.to_lowercase();
    if lower == prefix {
        return Some(0);
    }
    if lower.starts_with(prefix) {
        return Some(1);
    }
    let mut prev: Option<char> = None;
    for (i, ch) in candidate.char_indices() {
        let word_start = match prev {
            None => false,
            Some(p) => !p.is_alphanumeric() || (p.is_lowercase() && ch.is_uppercase()),
        };
        if word_start
            && ch.is_alphanumeric()
            && candidate
                .get(i..)
                .is_some_and(|rest| rest.to_lowercase().starts_with(prefix))
        {
            return Some(2);
        }
        prev = Some(ch);
    }
    lower.contains(prefix).then_some(3)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(c: &Completions) -> Vec<&str> {
        c.items.iter().map(|i| i.label.as_str()).collect()
    }

    /// The completions with the cursor at the `|` of `marked`.
    fn at(marked: &str, others: &[FileText<'_>]) -> Completions {
        let offset = marked.find('|').unwrap();
        let text = marked.replacen('|', "", 1);
        complete(&text, offset, others, 1000)
    }

    fn hover_at(marked: &str, others: &[FileText<'_>]) -> Option<Hover> {
        let offset = marked.find('|').unwrap();
        let text = marked.replacen('|', "", 1);
        hover(&text, offset, others)
    }

    #[test]
    fn keywords_of_the_context() {
        let c = at("table T\n\tcolumn C\n\t\tdataT|", &[]);
        assert_eq!(labels(&c)[0], "dataType");
        assert_eq!(c.prefix, "dataT");
        assert_eq!(c.items[0].insert_text, "dataType: ");
        assert_eq!(c.items[0].kind, ItemKind::Property);
        assert_eq!(c.items[0].detail, "dataType: <value>, dataType = <value>");
        let d = c.items[0].description.as_deref().unwrap();
        assert!(
            d.starts_with("Property of `column`. Takes a DataType value: "),
            "{d}"
        );

        // A blank line under a table: its keywords, objects as `keyword `.
        let c = at("table T\n\t|\n", &[]);
        let column = c.items.iter().find(|i| i.label == "column").unwrap();
        assert_eq!(column.insert_text, "column ");
        assert_eq!(column.kind, ItemKind::Object);
        assert!(labels(&c).contains(&"lineageTag"));
        assert!(!labels(&c).contains(&"dataType"));

        // The top level: objects and `ref`.
        let c = at("|", &[]);
        assert!(labels(&c).contains(&"table"));
        assert!(labels(&c).contains(&"ref"));

        // A word mid-line keeps what follows it.
        let c = at("table T\n\tcolumn C\n\t\tdata|Type: string", &[]);
        let item = c.items.iter().find(|i| i.label == "dataType").unwrap();
        assert_eq!(item.insert_text, "dataType");
        assert_eq!((c.start, c.end), (20, 28));
    }

    #[test]
    fn siblings_shape_the_keywords() {
        // A property set already is not offered again (the cursor's own line aside).
        let c = at("table T\n\tcolumn C\n\t\tdataType: string\n\t\t|", &[]);
        assert!(!labels(&c).contains(&"dataType"));
        let c = at("table T\n\tcolumn C\n\t\tdataT|ype: string", &[]);
        assert!(labels(&c).contains(&"dataType"));
        // After a child object only objects (TMDL109); before a property only properties.
        let c = at("table T\n\tcolumn C\n\t\tannotation A = 1\n\t\t|", &[]);
        assert!(labels(&c).contains(&"annotation"));
        assert!(!labels(&c).contains(&"dataType"));
        let c = at("table T\n\tcolumn C\n\t\t|\n\t\tdataType: string\n", &[]);
        assert!(labels(&c).contains(&"isHidden"));
        assert!(!labels(&c).contains(&"annotation"));
    }

    #[test]
    fn variants_and_levels() {
        // `column X = …` is a calculated column; a column of a calculated table its own context.
        let c = at("table T\n\tcolumn C = 1\n\t\t|", &[]);
        assert!(!labels(&c).contains(&"sourceColumn"));
        assert!(labels(&c).contains(&"formatString"));
        let c = at(
            "table T\n\tcolumn C\n\t\t|\n\tpartition P = calculated\n\t\tsource = 1\n",
            &[],
        );
        assert!(labels(&c).contains(&"isNameInferred"));
        assert!(!labels(&at("table T\n\tcolumn C\n\t\t|", &[])).contains(&"isNameInferred"));

        // Under a property, a ref line or an unknown keyword: nothing.
        assert!(at("table T\n\tlineageTag: x\n\t\t|", &[]).items.is_empty());
        assert!(at("table T\n\tbogus\n\t\t|", &[]).items.is_empty());
        // In an expression, a comment, past a declared name: nothing.
        assert!(
            at("table T\n\tmeasure M =\n\t\t\tSU|M(1)", &[])
                .items
                .is_empty()
        );
        assert!(at("// tab|", &[]).items.is_empty());
        assert!(at("table T|x\n", &[]).items.is_empty());

        // The database's level hides keywords that need a higher one.
        let low = [FileText {
            path: "database.tmdl",
            text: "database\n\tcompatibilityLevel: 1200\n",
        }];
        let high = [FileText {
            path: "database.tmdl",
            text: "database\n\tcompatibilityLevel: 1600\n",
        }];
        let needs = catalog()
            .context("table")
            .unwrap()
            .keywords
            .iter()
            .find(|k| k.min_level().is_some_and(|l| l > 1200 && l <= 1600))
            .unwrap()
            .keyword;
        assert!(!labels(&at("table T\n\t|", &low)).contains(&needs));
        assert!(labels(&at("table T\n\t|", &high)).contains(&needs));
        assert!(labels(&at("table T\n\t|", &[])).contains(&needs));
    }

    #[test]
    fn values() {
        let c = at("table T\n\tcolumn C\n\t\tdataType: |", &[]);
        assert!(labels(&c).contains(&"string"));
        assert!(labels(&c).contains(&"int64"));
        assert!(
            c.items
                .iter()
                .all(|i| i.kind == ItemKind::Value && i.detail == "DataType")
        );
        let c = at("table T\n\tcolumn C\n\t\tdataType: dou|", &[]);
        assert_eq!(labels(&c), ["double"]);
        assert_eq!((c.start, c.end), (30, 33));
        let c = at("table T\n\tcolumn C\n\t\tisHidden: |", &[]);
        assert_eq!(labels(&c), ["false", "true"]);
        // Text values: nothing.
        assert!(
            at("table T\n\tcolumn C\n\t\tlineageTag: |", &[])
                .items
                .is_empty()
        );
    }

    const SALES: &str = "/// What was sold.\ntable Sales\n\tmeasure 'Total Sales' = 1\n\tcolumn Amount\n\tcolumn 'Order Date'\n\thierarchy Dates\n";
    const MODEL: &str = "model Model\n\tculture: en-US\n\nqueryGroup Facts\n\nref table Sales\n";

    #[test]
    fn references_across_files() {
        let others = [
            FileText {
                path: "tables/Sales.tmdl",
                text: SALES,
            },
            FileText {
                path: "model.tmdl",
                text: MODEL,
            },
        ];
        // A column of the line's own table.
        let c = at("table Sales\n\tcolumn Year\n\t\tsortByColumn: |", &others);
        assert_eq!(labels(&c), ["'Order Date'", "Amount", "Year"]);
        assert_eq!(c.items[0].detail, "column of Sales");
        // Table.Column paths, quoted as TMDL writes them; quotes are ignored when matching.
        let c = at("relationship r\n\tfromColumn: 'sales'.ord|", &others);
        assert_eq!(labels(&c), ["Sales.'Order Date'"]);
        assert_eq!(c.items[0].kind, ItemKind::Column);
        // Query groups, tables of a ref line or a role's permission, the model's measures.
        let c = at("table T\n\tpartition P = m\n\t\tqueryGroup: |", &others);
        assert_eq!(labels(&c), ["Facts"]);
        assert_eq!(labels(&at("ref table |", &others)), ["Sales"]);
        let c = at("role R\n\ttablePermission S|", &others);
        assert_eq!(labels(&c), ["Sales"]);
        assert_eq!(c.items[0].description.as_deref(), Some("What was sold."));
        let c = at(
            "role R\n\ttablePermission Sales\n\t\tcolumnPermission |",
            &others,
        );
        assert_eq!(labels(&c), ["'Order Date'", "Amount"]);
        let c = at("model M\n\tdefaultMeasure: tot|", &others);
        assert_eq!(labels(&c), ["'Total Sales'"]);
        // The object types of a ref line.
        let c = at("ref |", &others);
        // Desktop writes `ref cultureInfo`, the TOM library `ref culture`.
        assert!(labels(&c).contains(&"cultureInfo"));
        assert!(labels(&c).contains(&"culture"));
        assert_eq!(c.items[0].insert_text, format!("{} ", c.items[0].label));
        // A declared name is not completed.
        assert!(at("table Sa|", &others).items.is_empty());
    }

    #[test]
    fn ranking_and_limits() {
        // The same word, then words that start with it, by label.
        let c = at("table T\n\tcolumn C\n\t\tType|", &[]);
        assert_eq!(
            labels(&c),
            [
                "type",
                "dataType",
                "isDataTypeInferred",
                "sourceProviderType"
            ]
        );
        let all = at("table T\n\t|", &[]);
        let some = {
            let text = "table T\n\t";
            complete(text, text.len(), &[], 3)
        };
        assert_eq!(some.items.len(), 3);
        assert!(some.incomplete);
        assert!(!all.incomplete);
        assert_eq!(&labels(&all)[..3], &labels(&some)[..]);
        assert_eq!(quality("sortByColumn", "by"), Some(2));
        assert_eq!(quality("'Order Date'", "date"), Some(2));
        assert_eq!(quality("lineageTag", "eage"), Some(3));
        assert_eq!(quality("lineageTag", "x"), None);
    }

    #[test]
    fn hovers() {
        let others = [
            FileText {
                path: "tables/Sales.tmdl",
                text: SALES,
            },
            FileText {
                path: "model.tmdl",
                text: MODEL,
            },
        ];
        let h = hover_at("table T\n\tcolumn C\n\t\tdata|Type: string", &others).unwrap();
        assert_eq!((h.kind, h.name.as_str()), (ItemKind::Property, "dataType"));
        assert!(
            h.markdown
                .starts_with("**`dataType`**: property of `column`\n\nWritten `dataType: <value>`"),
            "{}",
            h.markdown
        );
        let h = hover_at("table T\n\tcolumn C\n\t\tdataType: str|ing", &others).unwrap();
        assert_eq!((h.kind, h.name.as_str()), (ItemKind::Value, "string"));
        assert!(h.markdown.contains("enumeration `DataType`"));

        let h = hover_at("ref table Sa|les", &others).unwrap();
        assert_eq!((h.kind, h.name.as_str()), (ItemKind::Table, "Sales"));
        assert_eq!(
            h.declaration,
            Some(Declaration {
                file: Some(0),
                line: 2
            })
        );
        assert_eq!(
            h.markdown,
            "**table** `Sales`\n\nWhat was sold.\n\n```\ntable Sales\n```\n\nDeclared in `tables/Sales.tmdl`, line 2."
        );
        let h = hover_at("relationship r\n\ttoColumn: Sales.'Order |Date'", &others).unwrap();
        assert_eq!(
            (h.kind, h.name.as_str()),
            (ItemKind::Column, "Sales.'Order Date'")
        );
        let h = hover_at(
            "table T\n\tpartition P = m\n\t\tqueryGroup: Fac|ts",
            &others,
        )
        .unwrap();
        assert_eq!(h.kind, ItemKind::Name);
        assert!(h.markdown.starts_with("**query group** `Facts`"));
        // Declared in the text itself.
        let h = hover_at("table A\n\tcolumn B\n\tcolumn C\n\t\tsortByColumn: |B", &[]).unwrap();
        assert_eq!(
            h.declaration,
            Some(Declaration {
                file: None,
                line: 2
            })
        );
        assert!(h.markdown.ends_with("Declared in this file, line 2."));

        assert!(hover_at("table T\n\tcolumn C\n\t\tdataType: |", &others).is_none());
        assert!(hover_at("ref table Nope|", &others).is_none());
        assert!(hover_at("table T\n\tmeasure M = SU|M(1)", &others).is_none());
        assert!(hover_at("table T\n\tbo|gus", &others).is_none());
    }

    #[test]
    fn offsets_inside_characters_are_floored() {
        let text = "table 'Café'\n\t";
        for offset in 0..=text.len() + 2 {
            let c = complete(text, offset, &[], 10);
            assert!(text.is_char_boundary(c.start) && text.is_char_boundary(c.end));
            let _ = hover(text, offset, &[]);
        }
    }
}
