//! Report checks: visual fields (LINT001/LINT002), `queryRef`s (LINT003),
//! filters (LINT004), sorts (LINT005), `visualInteractions` (LINT006),
//! dataset references (LINT012), duplicate names (LINT014) and layout
//! (LINT015/LINT016).

use std::collections::{HashMap, HashSet};

use pbi_model::{
    AggregationFunction, DatasetReference, FieldExpr, Filter, Model, Origin, Page, Projection,
    Report, Severity, Visual, dax_column_ref, dax_measure_ref, names_eq,
};
use serde_json::Value;

use super::{Binding, Finding, Sink, join, pointer_token, rules, suggest};

/// Report-level ("extension") measures: `(table, measure)`, lowercased.
pub(crate) type ExtensionMeasures = HashSet<(String, String)>;

/// Visual types treated as decoration by the overlap rule.
const DECORATIVE: &[&str] = &["shape", "basicShape", "image"];

/// Tolerance for layout comparisons, in page units.
const EPS: f64 = 0.5;

/// Pages with more visuals than this skip the (quadratic) overlap rule.
const MAX_OVERLAP_VISUALS: usize = 1000;

/// Maximum JSON depth walked inside `objects`.
const MAX_JSON_DEPTH: usize = 64;

pub(crate) fn check_report(
    sink: &mut Sink<'_>,
    report: &Report,
    folder: &str,
    binding: Binding<'_>,
    ext: &ExtensionMeasures,
) {
    let pbir = Origin::file(join(folder, "definition.pbir"));
    let model = match binding {
        Binding::Model(m) => Some(m),
        Binding::Remote => {
            sink.push(
                Finding::new(
                    rules::DATASET_REF,
                    Severity::Info,
                    format!(
                        "report `{}` uses a remote semantic model (byConnection); field references not checked",
                        report.name
                    ),
                )
                .at(Some(pbir))
                .on(report.id),
            );
            sink.note(format!(
                "report `{}`: remote model, field references not checked",
                report.name
            ));
            None
        }
        Binding::Unresolved => {
            let path = match &report.dataset_reference {
                DatasetReference::ByPath { path } => path.as_str(),
                _ => "",
            };
            sink.push(
                Finding::new(
                    rules::DATASET_REF,
                    Severity::Error,
                    format!(
                        "report `{}` references semantic model `{path}` (byPath), which does not resolve to a semantic model folder",
                        report.name
                    ),
                )
                .at(Some(pbir.with_pointer("/datasetReference/byPath/path")))
                .on(report.id)
                .suggest(Some(
                    "point byPath at the `.SemanticModel` folder, relative to the report folder"
                        .to_owned(),
                )),
            );
            sink.note(format!(
                "report `{}`: model not found, field references not checked",
                report.name
            ));
            None
        }
        Binding::Missing => {
            sink.note(format!(
                "report `{}`: semantic model not loaded, field references not checked",
                report.name
            ));
            None
        }
    };
    let ctx = Ctx { model, ext };

    for f in &report.filters {
        ctx.filter(sink, f, "report filter");
    }
    duplicate_pages(sink, report);
    for page in &report.pages {
        ctx.page(sink, page);
    }
}

fn duplicate_pages(sink: &mut Sink<'_>, report: &Report) {
    let mut names: HashSet<&str> = HashSet::new();
    let mut display: HashSet<String> = HashSet::new();
    for p in &report.pages {
        if !p.name.is_empty() && !names.insert(p.name.as_str()) {
            sink.push(
                Finding::new(
                    rules::DUPLICATE_NAME,
                    Severity::Error,
                    format!("report `{}` has two pages named `{}`", report.name, p.name),
                )
                .at(p.origin.clone())
                .on(p.id),
            );
        }
        if !p.display_name.is_empty() && !display.insert(p.display_name.to_lowercase()) {
            sink.push(
                Finding::new(
                    rules::DUPLICATE_NAME,
                    Severity::Warning,
                    format!(
                        "report `{}` has two pages titled `{}`",
                        report.name, p.display_name
                    ),
                )
                .at(p.origin.clone())
                .on(p.id)
                .suggest(Some("rename one of the pages".to_owned())),
            );
        }
    }
}

/// A broken field reference.
struct Problem {
    /// The table itself is missing (LINT001 rather than LINT002).
    missing_table: bool,
    message: String,
    suggestion: Option<String>,
}

impl Problem {
    fn table(model: &Model, entity: &str, property: Option<&str>) -> Self {
        let suggestion = suggest::table(model, entity).or_else(|| {
            let property = property?;
            let owners: Vec<&str> = model
                .tables
                .iter()
                .filter(|t| t.column(property).is_some() || t.measure(property).is_some())
                .map(|t| t.name.as_str())
                .collect();
            match owners.as_slice() {
                [one] => Some(format!("`{property}` exists on table `{one}`")),
                _ => None,
            }
        });
        Self {
            missing_table: true,
            message: format!("table `{entity}` does not exist in the model"),
            suggestion,
        }
    }

    fn field(message: String, suggestion: Option<String>) -> Self {
        Self {
            missing_table: false,
            message,
            suggestion,
        }
    }
}

/// Resolves `field` against `model`; `None` if it is fine (or not a model
/// reference).
fn field_problem(model: &Model, field: &FieldExpr, ext: &ExtensionMeasures) -> Option<Problem> {
    match field.base() {
        FieldExpr::Column { entity, property } => {
            let Some(t) = model.table(entity) else {
                return Some(Problem::table(model, entity, Some(property)));
            };
            if t.column(property).is_some() {
                return None;
            }
            let r = dax_column_ref(&t.name, property);
            Some(if t.measure(property).is_some() {
                Problem::field(
                    format!("`{r}` is bound as a column but is a measure"),
                    Some("bind it as a Measure".to_owned()),
                )
            } else {
                Problem::field(
                    format!("column `{r}` does not exist"),
                    suggest::field(model, &t.name, property),
                )
            })
        }
        FieldExpr::Measure { entity, property } => {
            if ext.contains(&(entity.to_lowercase(), property.to_lowercase())) {
                return None;
            }
            let Some(t) = model.table(entity) else {
                return Some(Problem::table(model, entity, Some(property)));
            };
            if t.measure(property).is_some() {
                return None;
            }
            let r = dax_column_ref(&t.name, property);
            Some(if t.column(property).is_some() {
                Problem::field(
                    format!("`{r}` is bound as a measure but is a column"),
                    Some("bind it as a Column (with an aggregation)".to_owned()),
                )
            } else if let Some((home, m)) = model.find_measure(property) {
                Problem::field(
                    format!(
                        "measure `{}` is not on table `{}` (it is on `{}`)",
                        dax_measure_ref(&m.name),
                        t.name,
                        home.name
                    ),
                    Some(format!("set the SourceRef entity to `{}`", home.name)),
                )
            } else {
                Problem::field(
                    format!("measure `{r}` does not exist"),
                    suggest::field(model, &t.name, property),
                )
            })
        }
        FieldExpr::HierarchyLevel {
            entity,
            hierarchy,
            level,
            variation,
        } => {
            let Some(t) = model.table(entity) else {
                return Some(Problem::table(model, entity, None));
            };
            if let Some(v) = variation {
                // Auto date/time: the hierarchy lives in a hidden local date
                // table; only the column the variation hangs off is checked.
                return t.column(&v.property).is_none().then(|| {
                    Problem::field(
                        format!(
                            "column `{}` does not exist",
                            dax_column_ref(&t.name, &v.property)
                        ),
                        suggest::field(model, &t.name, &v.property),
                    )
                });
            }
            let Some(h) = t.hierarchy(hierarchy) else {
                return Some(Problem::field(
                    format!(
                        "hierarchy `{hierarchy}` does not exist on table `{}`",
                        t.name
                    ),
                    suggest::closest(hierarchy, t.hierarchies.iter().map(|h| h.name.as_str()))
                        .map(|h| format!("did you mean `{h}`?")),
                ));
            };
            h.level(level).is_none().then(|| {
                Problem::field(
                    format!(
                        "level `{level}` does not exist in hierarchy `{}`.`{}`",
                        t.name, h.name
                    ),
                    suggest::closest(level, h.levels.iter().map(|l| l.name.as_str()))
                        .map(|l| format!("did you mean `{l}`?")),
                )
            })
        }
        _ => None,
    }
}

/// Human label of a visual: `cardVisual `d020…``, `group `Header``.
fn label(v: &Visual) -> String {
    if v.is_group() {
        format!("group `{}`", v.display_name.as_deref().unwrap_or(&v.name))
    } else if v.visual_type.is_empty() {
        format!("visual `{}`", v.name)
    } else {
        format!("{} `{}`", v.visual_type, v.name)
    }
}

/// Origin of a part of a visual: the PBIR JSON pointer `pointer` inside
/// `visual.json`, or the visual's own origin for legacy layouts (whose
/// visuals are embedded JSON strings).
fn visual_origin(v: &Visual, pointer: impl FnOnce() -> String) -> Option<Origin> {
    let o = v.origin.as_ref()?;
    Some(match o.pointer {
        None => o.clone().with_pointer(pointer()),
        Some(_) => o.clone(),
    })
}

fn page_origin(p: &Page, pointer: impl FnOnce() -> String) -> Option<Origin> {
    let o = p.origin.as_ref()?;
    Some(match o.pointer {
        None => o.clone().with_pointer(pointer()),
        Some(_) => o.clone(),
    })
}

struct Ctx<'a> {
    model: Option<&'a Model>,
    ext: &'a ExtensionMeasures,
}

impl Ctx<'_> {
    fn problem(&self, field: &FieldExpr) -> Option<Problem> {
        field_problem(self.model?, field, self.ext)
    }

    fn filter(&self, sink: &mut Sink<'_>, f: &Filter, scope: &str) {
        let Some(model) = self.model else {
            return;
        };
        let mut problems: Vec<Problem> = Vec::new();
        f.visit_fields(&mut |field| {
            if let Some(p) = field_problem(model, field, self.ext)
                && !problems.iter().any(|q| q.message == p.message)
            {
                problems.push(p);
            }
        });
        let name = f.display_name.as_deref().unwrap_or(&f.name);
        for p in problems {
            sink.push(
                Finding::new(
                    rules::FILTER_REF,
                    Severity::Error,
                    format!("{scope} `{name}`: {}", p.message),
                )
                .at(f.origin.clone())
                .on(f.id)
                .suggest(p.suggestion),
            );
        }
    }

    fn page(&self, sink: &mut Sink<'_>, page: &Page) {
        let title = if page.display_name.is_empty() {
            &page.name
        } else {
            &page.display_name
        };
        let mut names: HashSet<&str> = HashSet::new();
        for v in &page.visuals {
            if !v.name.is_empty() && !names.insert(v.name.as_str()) {
                sink.push(
                    Finding::new(
                        rules::DUPLICATE_NAME,
                        Severity::Error,
                        format!("page `{title}` has two visuals named `{}`", v.name),
                    )
                    .at(v.origin.clone())
                    .on(v.id),
                );
            }
        }
        for (i, vi) in page.visual_interactions.iter().enumerate() {
            for (side, name) in [("source", &vi.source), ("target", &vi.target)] {
                if page.visual(name).is_some() {
                    continue;
                }
                sink.push(
                    Finding::new(
                        rules::INTERACTION_REF,
                        Severity::Warning,
                        format!(
                            "page `{title}`: visualInteractions[{i}] {side} `{name}` is not a visual on this page"
                        ),
                    )
                    .at(page_origin(page, || format!("/visualInteractions/{i}/{side}")))
                    .on(page.id)
                    .suggest(Some(
                        "remove the interaction (its visual was deleted or renamed)".to_owned(),
                    )),
                );
            }
        }
        for f in &page.filters {
            self.filter(sink, f, &format!("page `{title}` filter"));
        }
        for v in &page.visuals {
            self.visual(sink, v);
        }
        if sink.enabled(rules::VISUAL_BOUNDS) {
            bounds(sink, page, title);
        }
        if sink.enabled(rules::VISUAL_OVERLAP) {
            overlaps(sink, page, title);
        }
    }

    fn visual(&self, sink: &mut Sink<'_>, v: &Visual) {
        let label = label(v);
        let mut role_index: HashMap<&str, usize> = HashMap::new();
        for p in &v.projections {
            let slot = role_index.entry(p.role.as_str()).or_default();
            let index = *slot;
            *slot += 1;
            let pointer = format!(
                "/visual/query/queryState/{}/projections/{index}",
                pointer_token(&p.role)
            );
            if let Some(problem) = self.problem(&p.field) {
                let rule = if problem.missing_table {
                    rules::MISSING_TABLE
                } else {
                    rules::MISSING_FIELD
                };
                sink.push(
                    Finding::new(
                        rule,
                        Severity::Error,
                        format!("{label} ({}): {}", p.role, problem.message),
                    )
                    .at(visual_origin(v, || format!("{pointer}/field")))
                    .on(v.id)
                    .suggest(problem.suggestion),
                );
            }
            query_refs(sink, v, p, &label, &pointer);
        }
        for (i, s) in v.sort.iter().enumerate() {
            if let Some(problem) = self.problem(&s.field) {
                sink.push(
                    Finding::new(
                        rules::SORT_REF,
                        Severity::Error,
                        format!("{label} sort: {}", problem.message),
                    )
                    .at(visual_origin(v, || {
                        format!("/visual/query/sortDefinition/sort/{i}")
                    }))
                    .on(v.id)
                    .suggest(problem.suggestion),
                );
            }
        }
        for f in &v.filters {
            self.filter(sink, f, &format!("{label} filter"));
        }
        let objects_base = if v.is_group() {
            "/visualGroup/objects"
        } else {
            "/visual/objects"
        };
        let mut refs = JsonRefs::default();
        let mut ptr = objects_base.to_owned();
        walk_json(&v.objects, &mut ptr, 0, &mut refs);
        let mut ptr = "/visual/visualContainerObjects".to_owned();
        walk_json(&v.container_objects, &mut ptr, 0, &mut refs);
        for (pointer, field) in &refs.fields {
            if let Some(problem) = self.problem(field) {
                let rule = if problem.missing_table {
                    rules::MISSING_TABLE
                } else {
                    rules::MISSING_FIELD
                };
                sink.push(
                    Finding::new(
                        rule,
                        Severity::Error,
                        format!("{label} formatting: {}", problem.message),
                    )
                    .at(visual_origin(v, || pointer.clone()))
                    .on(v.id)
                    .suggest(problem.suggestion),
                );
            }
        }
        if !v.projections.is_empty() {
            for (pointer, metadata) in &refs.metadata {
                if v.projections
                    .iter()
                    .any(|p| names_eq(&p.query_ref, metadata))
                {
                    continue;
                }
                let close =
                    suggest::closest(metadata, v.projections.iter().map(|p| p.query_ref.as_str()));
                sink.push(
                    Finding::new(
                        rules::STALE_QUERY_REF,
                        Severity::Info,
                        format!(
                            "{label}: formatting selector `{metadata}` matches no projection queryRef, so it is ignored"
                        ),
                    )
                    .at(visual_origin(v, || pointer.clone()))
                    .on(v.id)
                    .suggest(close.map(|c| format!("did you mean `{c}`?"))),
                );
            }
        }
    }
}

/// How a `queryRef` compares with the one generated from the field.
#[derive(Debug, PartialEq, Eq)]
enum RefMatch {
    Same,
    /// Same field, different aggregation function name.
    Aggregation,
    Different,
}

fn split_call(s: &str) -> Option<(&str, &str)> {
    let open = s.find('(')?;
    let inner = s.get(open + 1..)?.strip_suffix(')')?;
    Some((s.get(..open)?.trim(), inner))
}

fn compare_query_ref(field: &FieldExpr, actual: &str, expected: &str) -> RefMatch {
    if names_eq(actual, expected) {
        return RefMatch::Same;
    }
    // Switching a field between "Don't summarize" and an aggregation keeps
    // the original queryRef (`Sum(T.C)` on a plain column, or the reverse).
    if !matches!(field, FieldExpr::Aggregation { .. })
        && let Some((_, inner)) = split_call(actual)
        && compare_query_ref(field, inner, expected) == RefMatch::Same
    {
        return RefMatch::Same;
    }
    match field {
        FieldExpr::HierarchyLevel { entity, level, .. } => {
            let lower = actual.to_lowercase();
            if lower.starts_with(&format!("{}.", entity.to_lowercase()))
                && lower.ends_with(&format!(".{}", level.to_lowercase()))
            {
                return RefMatch::Same;
            }
        }
        FieldExpr::Aggregation { expr, function } => {
            if let (Some((name, inner)), Some(want)) = (split_call(actual), expr.query_ref())
                && compare_query_ref(expr, inner, &want) == RefMatch::Same
            {
                let same_fn = AggregationFunction::parse_name(name) == Some(*function)
                    || function.query_ref_name().is_some_and(|n| names_eq(n, name));
                return if same_fn {
                    RefMatch::Same
                } else {
                    RefMatch::Aggregation
                };
            }
            if compare_query_ref(expr, actual, &expr.query_ref().unwrap_or_default())
                == RefMatch::Same
            {
                return RefMatch::Same;
            }
        }
        _ => {}
    }
    RefMatch::Different
}

/// `true` if `native` is the field name, `<Aggregation> of <field>` or (for
/// hierarchy levels) the level name.
fn native_matches(field: &FieldExpr, native: &str) -> bool {
    let Some(property) = field.property() else {
        return true;
    };
    if names_eq(native, property) {
        return true;
    }
    let native = native.to_lowercase();
    let property = property.to_lowercase();
    native.ends_with(&format!(" of {property}"))
        || matches!(field.base(), FieldExpr::HierarchyLevel { .. }) && native.ends_with(&property)
}

fn query_refs(sink: &mut Sink<'_>, v: &Visual, p: &Projection, label: &str, pointer: &str) {
    let Some(expected) = p.field.query_ref() else {
        return;
    };
    let duplicates = v
        .projections
        .iter()
        .filter(|q| q.field.query_ref().as_deref() == Some(expected.as_str()))
        .count();
    let actual = p.query_ref.as_str();
    let finding = |severity, message: String| {
        Finding::new(rules::STALE_QUERY_REF, severity, message)
            .at(visual_origin(v, || format!("{pointer}/queryRef")))
            .on(v.id)
            .suggest(Some(format!("set queryRef to `{expected}`")))
    };
    if actual.is_empty() {
        sink.push(finding(
            Severity::Warning,
            format!("{label} ({}): projection has no queryRef", p.role),
        ));
    } else {
        // Power BI numbers repeated fields: `T.C`, `T.C1`, `T.C2`, …
        let unnumbered = actual.trim_end_matches(|c: char| c.is_ascii_digit());
        let numbered_duplicate = duplicates > 1
            && unnumbered.len() < actual.len()
            && compare_query_ref(&p.field, unnumbered, &expected) == RefMatch::Same;
        match compare_query_ref(&p.field, actual, &expected) {
            RefMatch::Same => {}
            _ if numbered_duplicate => {}
            RefMatch::Aggregation => sink.push(finding(
                Severity::Info,
                format!(
                    "{label} ({}): queryRef `{actual}` names a different aggregation than the field (`{expected}`)",
                    p.role
                ),
            )),
            RefMatch::Different => sink.push(finding(
                Severity::Warning,
                format!(
                    "{label} ({}): queryRef `{actual}` does not match the field `{expected}` (stale queryRef)",
                    p.role
                ),
            )),
        }
    }
    if let Some(native) = &p.native_query_ref
        && let Some(property) = p.field.property()
        && !native_matches(&p.field, native)
    {
        sink.push(
            Finding::new(
                rules::STALE_QUERY_REF,
                Severity::Info,
                format!(
                    "{label} ({}): nativeQueryRef `{native}` does not match the field name `{property}`",
                    p.role
                ),
            )
            .at(visual_origin(v, || format!("{pointer}/nativeQueryRef")))
            .on(v.id)
            .suggest(Some(format!("set nativeQueryRef to `{property}`"))),
        );
    }
}

/// Field references and selector `metadata` found in `objects` JSON.
#[derive(Default)]
struct JsonRefs {
    fields: Vec<(String, FieldExpr)>,
    metadata: Vec<(String, String)>,
}

/// `{"Expression": {"SourceRef": {"Entity": …}}, "Property": …}` of a
/// `Column`/`Measure` node.
fn json_field(kind: &str, v: &Value) -> Option<FieldExpr> {
    let property = v.get("Property")?.as_str()?;
    let entity = v
        .get("Expression")?
        .get("SourceRef")?
        .get("Entity")?
        .as_str()?;
    Some(if kind == "Column" {
        FieldExpr::column(entity, property)
    } else {
        FieldExpr::measure(entity, property)
    })
}

fn walk_json(v: &Value, ptr: &mut String, depth: usize, out: &mut JsonRefs) {
    if depth >= MAX_JSON_DEPTH {
        return;
    }
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                let len = ptr.len();
                ptr.push('/');
                ptr.push_str(&pointer_token(k));
                match k.as_str() {
                    "Column" | "Measure" => match json_field(k, child) {
                        Some(field) => out.fields.push((ptr.clone(), field)),
                        None => walk_json(child, ptr, depth + 1, out),
                    },
                    // Hierarchy levels and extension schemas are not checked.
                    "HierarchyLevel" => {}
                    "selector" => {
                        if let Some(m) = child.get("metadata").and_then(Value::as_str) {
                            out.metadata.push((format!("{ptr}/metadata"), m.to_owned()));
                        }
                        walk_json(child, ptr, depth + 1, out);
                    }
                    _ => walk_json(child, ptr, depth + 1, out),
                }
                ptr.truncate(len);
            }
        }
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let len = ptr.len();
                ptr.push('/');
                ptr.push_str(&i.to_string());
                walk_json(item, ptr, depth + 1, out);
                ptr.truncate(len);
            }
        }
        _ => {}
    }
}

fn bounds(sink: &mut Sink<'_>, page: &Page, title: &str) {
    for v in page.visuals.iter().filter(|v| !v.is_hidden) {
        let p = &v.position;
        let message = if !(p.width > 0.0 && p.height > 0.0) {
            format!(
                "page `{title}`: {} has zero size ({}×{})",
                label(v),
                p.width,
                p.height
            )
        } else if p.x < -EPS
            || p.y < -EPS
            || p.right() > page.width + EPS
            || p.bottom() > page.height + EPS
        {
            format!(
                "page `{title}`: {} at ({}, {}) size {}×{} extends outside the {}×{} page",
                label(v),
                p.x,
                p.y,
                p.width,
                p.height,
                page.width,
                page.height
            )
        } else {
            continue;
        };
        sink.push(
            Finding::new(rules::VISUAL_BOUNDS, Severity::Warning, message)
                .at(visual_origin(v, || "/position".to_owned()))
                .on(v.id)
                .suggest(Some("move or resize it inside the page".to_owned())),
        );
    }
}

fn overlaps(sink: &mut Sink<'_>, page: &Page, title: &str) {
    let boxes: Vec<&Visual> = page
        .visuals
        .iter()
        .filter(|v| {
            !v.is_group()
                && !v.is_hidden
                && !DECORATIVE.contains(&v.visual_type.as_str())
                && v.position.width > 0.0
                && v.position.height > 0.0
        })
        .collect();
    if boxes.len() > MAX_OVERLAP_VISUALS {
        sink.note(format!(
            "page `{title}`: more than {MAX_OVERLAP_VISUALS} visuals, overlap check skipped"
        ));
        return;
    }
    for (i, a) in boxes.iter().enumerate() {
        for b in boxes.iter().skip(i + 1) {
            if a.parent_group_name.is_some() && a.parent_group_name == b.parent_group_name {
                continue;
            }
            let (pa, pb) = (&a.position, &b.position);
            let overlap = pa.x < pb.right() - EPS
                && pb.x < pa.right() - EPS
                && pa.y < pb.bottom() - EPS
                && pb.y < pa.bottom() - EPS;
            if overlap {
                sink.push(
                    Finding::new(
                        rules::VISUAL_OVERLAP,
                        Severity::Info,
                        format!("page `{title}`: {} overlaps {}", label(a), label(b)),
                    )
                    .at(visual_origin(a, || "/position".to_owned()))
                    .on(a.id),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_ref_comparison_follows_pbir_conventions() {
        let col = FieldExpr::column("Stores", "Region");
        assert_eq!(
            compare_query_ref(&col, "stores.region", "Stores.Region"),
            RefMatch::Same
        );
        let count = FieldExpr::aggregation(
            FieldExpr::column("Orders", "Status"),
            AggregationFunction::CountNonNull,
        );
        let expected = count.query_ref().unwrap();
        assert_eq!(
            compare_query_ref(&count, "CountNonNull(Orders.Status)", &expected),
            RefMatch::Same
        );
        assert_eq!(
            compare_query_ref(&count, "Sum(Orders.Status)", &expected),
            RefMatch::Aggregation
        );
        assert_eq!(
            compare_query_ref(&count, "Sum(Orders.State)", &expected),
            RefMatch::Different
        );
        assert_eq!(
            compare_query_ref(&count, "Orders.Status", &expected),
            RefMatch::Same
        );
        let quantity = FieldExpr::column("Orders", "Quantity");
        assert_eq!(
            compare_query_ref(&quantity, "Sum(Orders.Quantity)", "Orders.Quantity"),
            RefMatch::Same
        );
        assert_eq!(
            compare_query_ref(
                &FieldExpr::measure("Orders", "Total Sales"),
                "Orders.Total Sales count",
                "Orders.Total Sales"
            ),
            RefMatch::Different
        );
        let distinct = FieldExpr::aggregation(
            FieldExpr::column("T", "C"),
            AggregationFunction::DistinctCount,
        );
        assert_eq!(
            compare_query_ref(&distinct, "Count(T.C)", "Count(T.C)"),
            RefMatch::Same
        );
        let level = FieldExpr::HierarchyLevel {
            entity: "Sales".into(),
            hierarchy: "Date Hierarchy".into(),
            level: "Year".into(),
            variation: Some(pbi_model::PropertyVariation {
                name: "Variation".into(),
                property: "OrderDate".into(),
            }),
        };
        assert_eq!(
            compare_query_ref(
                &level,
                "Sales.OrderDate.Variation.Date Hierarchy.Year",
                &level.query_ref().unwrap()
            ),
            RefMatch::Same
        );
    }

    #[test]
    fn native_query_refs_allow_aggregation_labels() {
        let count = FieldExpr::aggregation(
            FieldExpr::column("Orders", "Region"),
            AggregationFunction::CountNonNull,
        );
        assert!(native_matches(&count, "Count of Region"));
        assert!(native_matches(&count, "Region"));
        let m = FieldExpr::measure("Orders", "Total Sales");
        assert!(!native_matches(&m, "Total Sales count"));
    }

    #[test]
    fn objects_walk_finds_fields_and_selectors() {
        let objects = serde_json::json!({
            "values": [{
                "properties": {"fontColor": {"solid": {"color": {"expr": {"Measure": {
                    "Expression": {"SourceRef": {"Entity": "Orders"}},
                    "Property": "Color/Measure"
                }}}}}},
                "selector": {"metadata": "Orders.Status"}
            }]
        });
        let mut refs = JsonRefs::default();
        let mut ptr = "/visual/objects".to_owned();
        walk_json(&objects, &mut ptr, 0, &mut refs);
        assert_eq!(
            refs.fields,
            [(
                "/visual/objects/values/0/properties/fontColor/solid/color/expr/Measure".to_owned(),
                FieldExpr::measure("Orders", "Color/Measure")
            )]
        );
        assert_eq!(
            refs.metadata,
            [(
                "/visual/objects/values/0/selector/metadata".to_owned(),
                "Orders.Status".to_owned()
            )]
        );
    }
}
