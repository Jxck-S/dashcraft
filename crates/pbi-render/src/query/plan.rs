//! Visual type → query shape, projections → planned fields (group-by
//! columns and value expressions with their metadata).

use pbi_model::names::names_eq;
use pbi_model::{
    AggregationFunction, DataType, FieldExpr, Model, Projection, SortDirection, SummarizeBy, Visual,
};

use crate::conditional::{CfSpec, input_key, meta_matches};
use crate::dataview::{ColumnMeta, FieldRef, FieldRefKind};

use super::filter::{ColumnRef, FilterBuilder};

/// How a visual queries and which DataView it gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// No query.
    Empty,
    /// Cards: measures → Single (one field) or a one-row table.
    Card,
    /// Slicer: distinct values of its columns + selection.
    Slicer,
    /// Grouped table (`tableEx`, multi-row card, unknown types).
    Table,
    /// `pivotTable`.
    Matrix,
    /// Category / series / values charts.
    Chart,
}

/// Visual types that never query.
const NO_QUERY: [&str; 7] = [
    "textbox",
    "shape",
    "basicShape",
    "actionButton",
    "image",
    "pageNavigator",
    "bookmarkNavigator",
];

pub(crate) fn shape_of(visual: &Visual) -> Shape {
    let t = visual.visual_type.as_str();
    if visual.is_group() || t.is_empty() || NO_QUERY.contains(&t) {
        return Shape::Empty;
    }
    match t {
        "card" | "cardVisual" => Shape::Card,
        _ if super::interaction::is_slicer_type(t) => Shape::Slicer,
        "pivotTable" | "matrix" => Shape::Matrix,
        "tableEx" | "table" | "multiRowCard" => Shape::Table,
        "funnel" | "treemap" | "gauge" | "kpi" | "map" | "filledMap" => Shape::Chart,
        _ if t.ends_with("Chart") => Shape::Chart,
        _ => Shape::Table,
    }
}

/// What a projected field is used for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoleKind {
    /// Category / axis / table column / slicer value.
    Category,
    /// Legend / series.
    Series,
    /// Matrix rows.
    Rows,
    /// Matrix columns.
    Columns,
    /// A value (measure or aggregation).
    Value,
}

fn role_kind(shape: Shape, role: &str, field: &FieldExpr, summarized: bool) -> RoleKind {
    let is_value_field = summarized
        || matches!(field.base(), FieldExpr::Measure { .. })
        || matches!(field, FieldExpr::Aggregation { .. });
    match shape {
        Shape::Card => RoleKind::Value,
        Shape::Empty | Shape::Slicer => RoleKind::Category,
        Shape::Table if is_value_field => RoleKind::Value,
        Shape::Table => RoleKind::Category,
        Shape::Matrix => match role {
            "Rows" => RoleKind::Rows,
            "Columns" => RoleKind::Columns,
            _ => RoleKind::Value,
        },
        Shape::Chart => match role {
            // `TrendLine` is the kpi's trend axis (the indicator and goal
            // show the last point of it).
            "Category" | "Axis" | "Details" | "Group" | "Rows" | "Location" | "Play"
            | "TrendLine" => RoleKind::Category,
            "Series" | "Legend" | "ColorBy" | "Breakdown" => RoleKind::Series,
            _ => RoleKind::Value,
        },
    }
}

/// The value expression of a field.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ValueExpr {
    /// DAX scalar expression (`'Orders'[Total Sales]`, `SUM(…)`).
    pub dax: String,
    /// DAX shown on errors: the measure body, or `dax`.
    pub source_dax: String,
    /// Field label for error messages.
    pub label: String,
}

/// How a field is queried.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum FieldPlan {
    Group(ColumnRef),
    Value(ValueExpr),
}

/// A projected field.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlannedField {
    pub meta: ColumnMeta,
    pub kind: RoleKind,
    pub plan: FieldPlan,
    /// The projection's field (for sort matching and cross-filtering).
    pub field: FieldExpr,
}

impl PlannedField {
    pub(crate) fn group(&self) -> Option<&ColumnRef> {
        match &self.plan {
            FieldPlan::Group(c) => Some(c),
            FieldPlan::Value(_) => None,
        }
    }

    pub(crate) fn value(&self) -> Option<&ValueExpr> {
        match &self.plan {
            FieldPlan::Value(v) => Some(v),
            FieldPlan::Group(_) => None,
        }
    }
}

/// A field that could not be planned.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct FieldError {
    pub field: String,
    pub message: String,
}

/// Projections to query: per role, only the `active` ones when any is
/// marked active (drill level), in projection order.
pub(crate) fn active_projections(visual: &Visual) -> Vec<&Projection> {
    let mut out = Vec::new();
    for role in visual.roles() {
        let ps: Vec<&Projection> = visual.projections_for(role).collect();
        let any_active = ps.iter().any(|p| p.active == Some(true));
        out.extend(
            ps.into_iter()
                .filter(|p| !any_active || p.active == Some(true)),
        );
    }
    out
}

/// Plans every projection; the same field in several roles gives one
/// field with several roles.
pub(crate) fn plan_fields(
    model: &Model,
    builder: &FilterBuilder<'_>,
    visual: &Visual,
    shape: Shape,
) -> Result<Vec<PlannedField>, FieldError> {
    let mut out: Vec<PlannedField> = Vec::new();
    for p in active_projections(visual) {
        let is_column = matches!(
            p.field,
            FieldExpr::Column { .. } | FieldExpr::HierarchyLevel { .. }
        );
        // A plain column projected as `Sum(T.C)` is summarized by the visual
        // (PBIR keeps the column and records the aggregation in the queryRef).
        let hint = if is_column {
            query_ref_aggregation(&p.query_ref)
        } else {
            None
        };
        let summarized = hint.is_some()
            || (is_column
                && p.query_ref.is_empty()
                && shape == Shape::Table
                && builder
                    .resolve_column(&p.field)
                    .ok()
                    .and_then(|c| model.column(&c.table, &c.column))
                    .is_some_and(summarized_by_default));
        let kind = role_kind(shape, &p.role, &p.field, summarized);
        let query_ref = if p.query_ref.is_empty() {
            p.field.query_ref().unwrap_or_default()
        } else {
            p.query_ref.clone()
        };
        if let Some(existing) = out
            .iter_mut()
            .find(|f| f.meta.query_ref == query_ref && f.kind == kind)
        {
            if !existing.meta.roles.contains(&p.role) {
                existing.meta.roles.push(p.role.clone());
            }
            continue;
        }
        let label = p.label().to_owned();
        let err = |message: String| FieldError {
            field: label.clone(),
            message,
        };
        let planned = if kind == RoleKind::Value {
            let (mut expr, meta) =
                value_field_with(model, builder, &p.field, shape == Shape::Card, hint)
                    .map_err(err)?;
            expr.label.clone_from(&label);
            PlannedField {
                meta: ColumnMeta {
                    roles: vec![p.role.clone()],
                    query_ref,
                    display_name: label.clone(),
                    ..meta
                },
                kind,
                plan: FieldPlan::Value(expr),
                field: p.field.clone(),
            }
        } else {
            let col = builder.resolve_column(&p.field).map_err(err)?;
            let column = model.column(&col.table, &col.column);
            PlannedField {
                meta: ColumnMeta {
                    roles: vec![p.role.clone()],
                    query_ref,
                    display_name: label.clone(),
                    source: field_ref(&p.field, Some(&col), None),
                    is_measure: false,
                    format_string: column.and_then(|c| c.format_string.clone()),
                    data_type: Some(col.data_type.clone()),
                    expression: None,
                    sort: None,
                    sort_order: None,
                    hidden: false,
                },
                kind,
                plan: FieldPlan::Group(col),
                field: p.field.clone(),
            }
        };
        out.push(planned);
    }
    Ok(out)
}

/// Plans the inputs of the visual's conditional formats that are not
/// already value fields as hidden value fields (appended to `fields`, see
/// [`crate::dataview::ColumnMeta::hidden`]). Returns a message per input
/// that cannot be planned (its rules then do not apply).
pub(crate) fn plan_cf_inputs(
    model: &Model,
    builder: &FilterBuilder<'_>,
    spec: &CfSpec,
    fields: &mut Vec<PlannedField>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for input in spec.field_inputs() {
        let key = input_key(input);
        let known = fields.iter().any(|f| {
            f.kind == RoleKind::Value
                && if f.meta.hidden {
                    f.meta.query_ref == key
                } else {
                    meta_matches(&f.meta, input)
                }
        });
        if known {
            continue;
        }
        match value_field_with(model, builder, input, true, None) {
            Ok((mut expr, meta)) => {
                expr.label.clone_from(&key);
                fields.push(PlannedField {
                    meta: ColumnMeta {
                        roles: Vec::new(),
                        query_ref: key.clone(),
                        display_name: key,
                        hidden: true,
                        ..meta
                    },
                    kind: RoleKind::Value,
                    plan: FieldPlan::Value(expr),
                    field: input.clone(),
                });
            }
            Err(e) => problems.push(format!(
                "conditional formatting based on {key} not applied: {e}"
            )),
        }
    }
    problems
}

fn field_ref(
    field: &FieldExpr,
    column: Option<&ColumnRef>,
    aggregation: Option<AggregationFunction>,
) -> FieldRef {
    let (kind, hierarchy) = match field {
        FieldExpr::Column { .. } => (FieldRefKind::Column, None),
        FieldExpr::Measure { .. } => (FieldRefKind::Measure, None),
        FieldExpr::Aggregation { .. } => (FieldRefKind::Aggregation, None),
        FieldExpr::HierarchyLevel { hierarchy, .. } => {
            (FieldRefKind::HierarchyLevel, Some(hierarchy.clone()))
        }
        FieldExpr::Literal(_) | FieldExpr::Unknown(_) => (FieldRefKind::Other, None),
    };
    let kind = if aggregation.is_some() {
        FieldRefKind::Aggregation
    } else {
        kind
    };
    let entity = column
        .map(|c| c.table.clone())
        .or_else(|| field.entity().map(str::to_owned));
    let property = column
        .map(|c| c.column.clone())
        .or_else(|| field.property().map(str::to_owned));
    let source_name = match (&entity, &property) {
        (Some(t), Some(p)) => pbi_model::dax_column_ref(t, p),
        _ => field.query_ref().unwrap_or_default(),
    };
    FieldRef {
        kind,
        entity,
        property,
        aggregation,
        hierarchy,
        source_name,
    }
}

/// The value expression and metadata of a measure, aggregation or a column
/// summarized by default (see [`default_aggregation`]).
pub(crate) fn value_field(
    model: &Model,
    builder: &FilterBuilder<'_>,
    field: &FieldExpr,
    card: bool,
) -> Result<(ValueExpr, ColumnMeta), String> {
    value_field_with(model, builder, field, card, None)
}

/// [`value_field`] with the aggregation of a plain column given by the
/// projection (`hint`, from its queryRef).
pub(crate) fn value_field_with(
    model: &Model,
    builder: &FilterBuilder<'_>,
    field: &FieldExpr,
    card: bool,
    hint: Option<AggregationFunction>,
) -> Result<(ValueExpr, ColumnMeta), String> {
    let blank_meta = |source: FieldRef| ColumnMeta {
        roles: Vec::new(),
        query_ref: String::new(),
        display_name: String::new(),
        source,
        is_measure: true,
        format_string: None,
        data_type: None,
        expression: None,
        sort: None,
        sort_order: None,
        hidden: false,
    };
    match field {
        FieldExpr::Measure { entity, property } => {
            let (table, measure) = model
                .table(entity)
                .and_then(|t| t.measure(property).map(|m| (t, m)))
                .or_else(|| model.find_measure(property))
                .ok_or_else(|| format!("measure {entity}.{property} is not in the model"))?;
            let dax = pbi_model::dax_column_ref(&table.name, &measure.name);
            let mut meta = blank_meta(FieldRef {
                kind: FieldRefKind::Measure,
                entity: Some(table.name.clone()),
                property: Some(measure.name.clone()),
                aggregation: None,
                hierarchy: None,
                source_name: dax.clone(),
            });
            meta.format_string = measure.format_string.clone();
            meta.expression = Some(measure.expression.clone());
            Ok((
                ValueExpr {
                    dax,
                    source_dax: measure.expression.clone(),
                    label: measure.name.clone(),
                },
                meta,
            ))
        }
        FieldExpr::Aggregation { expr, function } => {
            let col = builder.resolve_column(expr)?;
            aggregation_field(model, field, &col, *function, &blank_meta)
        }
        FieldExpr::Column { .. } | FieldExpr::HierarchyLevel { .. } => {
            let col = builder.resolve_column(field)?;
            let summarize = model
                .column(&col.table, &col.column)
                .and_then(|c| c.summarize_by.clone());
            let function = hint
                .unwrap_or_else(|| default_aggregation(&col.data_type, summarize.as_ref(), card));
            aggregation_field(model, field, &col, function, &blank_meta)
        }
        FieldExpr::Literal(l) => Err(format!("literal {l} cannot be used as a value")),
        FieldExpr::Unknown(_) => Err("unsupported field expression".into()),
    }
}

fn aggregation_field(
    model: &Model,
    field: &FieldExpr,
    col: &ColumnRef,
    function: AggregationFunction,
    blank_meta: &dyn Fn(FieldRef) -> ColumnMeta,
) -> Result<(ValueExpr, ColumnMeta), String> {
    let dax = aggregation_dax(function, &col.dax())?;
    let mut meta = blank_meta(field_ref(field, Some(col), Some(function)));
    let column_format = model
        .column(&col.table, &col.column)
        .and_then(|c| c.format_string.clone());
    let (format, data_type) = match function {
        AggregationFunction::CountNonNull | AggregationFunction::DistinctCount => {
            (Some("0".to_owned()), Some(DataType::Int64))
        }
        AggregationFunction::Avg | AggregationFunction::StdDev | AggregationFunction::Var => {
            (column_format, Some(DataType::Double))
        }
        _ => (column_format, Some(col.data_type.clone())),
    };
    meta.format_string = format;
    meta.data_type = data_type;
    meta.expression = Some(dax.clone());
    Ok((
        ValueExpr {
            source_dax: dax.clone(),
            label: field.query_ref().unwrap_or_else(|| dax.clone()),
            dax,
        },
        meta,
    ))
}

/// DAX of an aggregation over a column reference.
///
/// # Errors
/// Unknown aggregation codes.
pub(crate) fn aggregation_dax(
    function: AggregationFunction,
    column: &str,
) -> Result<String, String> {
    let name = match function {
        AggregationFunction::Sum => "SUM",
        AggregationFunction::Avg => "AVERAGE",
        AggregationFunction::Min => "MIN",
        AggregationFunction::Max => "MAX",
        AggregationFunction::CountNonNull => "COUNTA",
        AggregationFunction::DistinctCount => "DISTINCTCOUNT",
        AggregationFunction::Median => "MEDIAN",
        AggregationFunction::StdDev => "STDEV.P",
        AggregationFunction::Var => "VAR.P",
        AggregationFunction::Unknown(code) => {
            return Err(format!("aggregation function code {code} is not supported"));
        }
    };
    Ok(format!("{name}({column})"))
}

/// Default summarization of a plain column in a value role (Power BI's
/// "implicit measure"): the model's `summarizeBy` when it is an
/// aggregation; otherwise `Sum` for numeric columns and `Count`
/// (`CountNonNull`) for text, dates, booleans and "don't summarize"
/// columns — except on cards, where a text column shows its first value
/// (`Min`).
pub(crate) fn default_aggregation(
    data_type: &DataType,
    summarize_by: Option<&SummarizeBy>,
    card: bool,
) -> AggregationFunction {
    match summarize_by {
        Some(SummarizeBy::Sum) => return AggregationFunction::Sum,
        Some(SummarizeBy::Min) => return AggregationFunction::Min,
        Some(SummarizeBy::Max) => return AggregationFunction::Max,
        Some(SummarizeBy::Count) => return AggregationFunction::CountNonNull,
        Some(SummarizeBy::Average) => return AggregationFunction::Avg,
        Some(SummarizeBy::DistinctCount) => return AggregationFunction::DistinctCount,
        _ => {}
    }
    let explicit_none = matches!(summarize_by, Some(SummarizeBy::None));
    if data_type.is_numeric() && !explicit_none {
        AggregationFunction::Sum
    } else if card && *data_type == DataType::String {
        AggregationFunction::Min
    } else {
        AggregationFunction::CountNonNull
    }
}

/// The aggregation named by a queryRef such as `Sum(Orders.Quantity)` or
/// `CountNonNull(T.C)` (Power BI's queryRef function names).
pub(crate) fn query_ref_aggregation(query_ref: &str) -> Option<AggregationFunction> {
    let (name, rest) = query_ref.trim().split_once('(')?;
    rest.strip_suffix(')')?;
    (0..9)
        .map(AggregationFunction::from_code)
        .find(|f| f.query_ref_name() == Some(name.trim()))
}

/// Tables without a queryRef (legacy layouts): numeric columns and columns
/// with an aggregating `summarizeBy` are summarized.
fn summarized_by_default(column: &pbi_model::Column) -> bool {
    match &column.summarize_by {
        Some(SummarizeBy::None) => false,
        Some(SummarizeBy::Default) | None => column.data_type.is_numeric(),
        Some(_) => true,
    }
}

/// `true` if `a` and `b` refer to the same field (names compared
/// case-insensitively).
pub(crate) fn same_field(a: &FieldExpr, b: &FieldExpr) -> bool {
    match (a, b) {
        (
            FieldExpr::Aggregation {
                expr: ea,
                function: fa,
            },
            FieldExpr::Aggregation {
                expr: eb,
                function: fb,
            },
        ) => fa == fb && same_field(ea, eb),
        (FieldExpr::Column { .. }, FieldExpr::Column { .. })
        | (FieldExpr::Measure { .. }, FieldExpr::Measure { .. })
        | (FieldExpr::HierarchyLevel { .. }, FieldExpr::HierarchyLevel { .. }) => {
            match (a.entity(), a.property(), b.entity(), b.property()) {
                (Some(t1), Some(p1), Some(t2), Some(p2)) => names_eq(t1, t2) && names_eq(p1, p2),
                _ => false,
            }
        }
        _ => a == b,
    }
}

/// A sort key over the rows of a grouped query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SortKey {
    /// Index of the field in the planned field list.
    pub field: usize,
    pub descending: bool,
}

/// The visual's sort keys (`sortDefinition`), matched to planned fields;
/// unmatched sort fields are returned by query ref.
pub(crate) fn sort_keys(visual: &Visual, fields: &[PlannedField]) -> (Vec<SortKey>, Vec<String>) {
    let mut keys = Vec::new();
    let mut missing = Vec::new();
    for s in &visual.sort {
        match fields.iter().position(|f| same_field(&f.field, &s.field)) {
            Some(i) => {
                if !keys.iter().any(|k: &SortKey| k.field == i) {
                    keys.push(SortKey {
                        field: i,
                        descending: s.direction == SortDirection::Descending,
                    });
                }
            }
            None => missing.push(s.field.query_ref().unwrap_or_else(|| "?".into())),
        }
    }
    (keys, missing)
}

/// Visual types whose default order is by the first value, descending.
/// Waterfalls are not: the service shows their categories in ascending
/// (chronological) order.
pub(crate) fn sorts_by_value_by_default(visual_type: &str) -> bool {
    let t = visual_type.to_ascii_lowercase();
    t.contains("bar")
        || t.contains("column")
        || t.contains("pie")
        || t.contains("donut")
        || t.contains("funnel")
        || t.contains("treemap")
        || t.contains("ribbon")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_types_map_to_shapes() {
        let shape = |t: &str| {
            shape_of(&Visual {
                visual_type: t.into(),
                ..Visual::default()
            })
        };
        assert_eq!(shape("textbox"), Shape::Empty);
        assert_eq!(shape("cardVisual"), Shape::Card);
        assert_eq!(shape("slicer"), Shape::Slicer);
        assert_eq!(shape("tableEx"), Shape::Table);
        assert_eq!(shape("pivotTable"), Shape::Matrix);
        assert_eq!(shape("clusteredBarChart"), Shape::Chart);
        assert_eq!(shape("pieChart"), Shape::Chart);
        for t in [
            "scatterChart",
            "funnel",
            "gauge",
            "treemap",
            "waterfallChart",
        ] {
            assert_eq!(shape(t), Shape::Chart, "{t}");
        }
        assert_eq!(shape("someCustomVisual"), Shape::Table);
    }

    #[test]
    fn kpi_trend_line_is_the_axis() {
        let field = FieldExpr::column("Date", "Year Month");
        let measure = FieldExpr::measure("Orders", "Total Sales");
        assert_eq!(
            role_kind(Shape::Chart, "TrendLine", &field, false),
            RoleKind::Category
        );
        assert_eq!(
            role_kind(Shape::Chart, "Indicator", &measure, false),
            RoleKind::Value
        );
        assert_eq!(
            role_kind(Shape::Chart, "Goal", &measure, false),
            RoleKind::Value
        );
    }

    #[test]
    fn implicit_measures_follow_power_bi_defaults() {
        use AggregationFunction as A;
        assert_eq!(default_aggregation(&DataType::Int64, None, false), A::Sum);
        assert_eq!(
            default_aggregation(&DataType::Int64, Some(&SummarizeBy::None), false),
            A::CountNonNull
        );
        assert_eq!(
            default_aggregation(&DataType::Double, Some(&SummarizeBy::Average), false),
            A::Avg
        );
        assert_eq!(
            default_aggregation(&DataType::String, None, false),
            A::CountNonNull
        );
        assert_eq!(default_aggregation(&DataType::String, None, true), A::Min);
        assert_eq!(
            aggregation_dax(A::CountNonNull, "T[c]").as_deref(),
            Ok("COUNTA(T[c])")
        );
        assert!(aggregation_dax(A::Unknown(42), "T[c]").is_err());
        assert_eq!(query_ref_aggregation("Sum(Orders.Quantity)"), Some(A::Sum));
        assert_eq!(
            query_ref_aggregation("CountNonNull(T.Status)"),
            Some(A::CountNonNull)
        );
        assert_eq!(query_ref_aggregation("Orders.Order count"), None);
        assert_eq!(query_ref_aggregation("Foo(T.C)"), None);
    }

    #[test]
    fn default_sorts() {
        assert!(sorts_by_value_by_default("clusteredBarChart"));
        assert!(sorts_by_value_by_default("pieChart"));
        assert!(sorts_by_value_by_default("funnel"));
        assert!(!sorts_by_value_by_default("lineChart"));
        assert!(!sorts_by_value_by_default("waterfallChart"));
        assert!(!sorts_by_value_by_default("scatterChart"));
    }
}
