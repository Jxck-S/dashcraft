//! Conditional formatting of tables, matrices and charts.
//!
//! Power BI stores conditional formatting in a visual's `objects` as
//! semantic-query expressions in place of a static value:
//!
//! | Format style | PBIR | Here |
//! |---|---|---|
//! | Gradient (color scale) | `{"FillRule": {"Input": <field>, "FillRule": {"linearGradient2" \| "linearGradient3": {"min", ["mid",] "max", "nullColoringStrategy"}}}}` | [`ColorRule::Gradient`] |
//! | Rules | `{"Conditional": {"Cases": [{"Condition", "Value"}], "DefaultValue"}}` | [`ColorRule::Rules`], [`IconSource::Rules`] |
//! | Field value | a `Measure`, `Aggregation` or `Column` expression whose value is a color (or an icon name) | [`ColorRule::FieldValue`], [`IconSource::FieldValue`] |
//! | Data bars | `dataBars: {positiveColor, negativeColor, axisColor, reverseDirection, hideText, minValue, maxValue}` | [`DataBars`] |
//! | Icons | `icon: {kind, layout, verticalAlignment, value: <rules or field>}` | [`IconRule`] |
//!
//! Colors sit under `{"solid": {"color": {"expr": …}}}`. Where they are
//! read:
//!
//! - `tableEx` / `pivotTable`: entries of the `values` and
//!   `columnFormatting` objects whose `selector.metadata` names the
//!   formatted column or value field (its query ref): `backColor`,
//!   `fontColor`, `dataBars`, `icon`. `selector.data[].dataViewWildcard
//!   .matchingOption` says which cells: `1` values (the default without a
//!   wildcard), `2` totals and subtotals, `0` both ([`ApplyTo`]).
//! - Bar, column, pie, donut, treemap, scatter and funnel charts without a
//!   series (legend) field: `dataPoint.fill`, for every value field or the
//!   one named by `selector.metadata`.
//!
//! [`CfSpec::parse`] reads them (and lists what it cannot apply in
//! [`CfSpec::diagnostics`]); the query layer fetches every
//! [`Input::Field`] that is not already a value of the visual as a hidden
//! field ([`crate::dataview::ColumnMeta::hidden`]); renderers evaluate the
//! rules per cell with [`evaluate_cells`] over a [`CellSource`].
//!
//! # Semantics
//!
//! - **Gradients**: the input's numeric value on a linear sRGB scale from
//!   `min` to `max` (through `mid` for three colors). Bounds without a
//!   `value` come from the data: the smallest / largest input over the
//!   formatted cells that are not totals (the middle defaults to their
//!   mean). Values outside the bounds take the end colors; a constant
//!   input takes the color half-way. BLANK follows
//!   `nullColoringStrategy`: `asZero` (default), `noColor`
//!   (`dontColor`, `blank`), `specificColor` (its `color`).
//! - **Rules**: cases in order, the first whose condition holds wins, else
//!   `DefaultValue`, else no color. Conditions are `Comparison`s (`=`,
//!   `>`, `>=`, `<`, `<=`), `And`, `Or`, `Not` over fields, literals,
//!   `Arithmetic` and the visual-wide `Min`, `Max` and `Percentile` of a
//!   field (optionally wrapped in a `ScopedEval` with an empty scope), which
//!   is how ranges relative to the data ("percent") are written. BLANK
//!   matches only a comparison with `null`; numbers compare numerically,
//!   text case-insensitively.
//! - **Field values**: CSS colors ([`color::parse_css_color`]): hex,
//!   `rgb[a]()`, `hsl[a]()`, color names. Icon field values are built-in
//!   icon names ([`Icon::parse`]); image URLs are not drawn.
//! - **Data bars**: the cell's own value between `minValue` (default: the
//!   smallest value, or 0 when all are positive) and `maxValue` (default:
//!   the largest, or 0 when all are negative); positive bars grow from the
//!   axis in `positiveColor`, negative ones in `negativeColor`, with an
//!   axis line where the range crosses zero; `reverseDirection` mirrors
//!   them; `hideText` ("show bar only") hides the value. Values only, never
//!   totals.
//! - **Icons**: drawn before (`layout` `Before`, default) or after the
//!   value, or alone (`IconOnly`), aligned to the `Top` (default),
//!   `Middle` or `Bottom` of the cell.
//!
//! Anything else (other fill rules, `scopeId` selectors, URL icons, series
//! charts) renders without conditional formatting and is reported as a
//! diagnostic, never as an error.

pub mod color;
mod eval;
pub mod icons;
mod parse;

use pbi_engine::Value;
use pbi_model::names::names_eq;
use pbi_model::{ComparisonKind, FieldExpr, LiteralValue, Visual};
use serde_json::Value as Json;

use crate::dataview::{ColumnMeta, FieldRefKind};
use crate::scene::{Color, Paint};
use crate::theme::ThemeResolver;

pub use eval::Stats;
pub use icons::{Icon, IconShape};

/// Which cells a rule formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyTo {
    /// Values only (`matchingOption` 1; the default).
    Values,
    /// Totals and subtotals only (`matchingOption` 2).
    Totals,
    /// Values and totals (`matchingOption` 0).
    Both,
}

impl ApplyTo {
    pub fn values(self) -> bool {
        matches!(self, ApplyTo::Values | ApplyTo::Both)
    }

    pub fn totals(self) -> bool {
        matches!(self, ApplyTo::Totals | ApplyTo::Both)
    }
}

/// What a rule reads.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    /// The formatted cell's own value (data bars).
    Own,
    /// A field of the model (a measure, an aggregation of a column or a
    /// column): a value of the visual or a hidden field.
    Field(FieldExpr),
}

/// One conditional format.
#[derive(Debug, Clone, PartialEq)]
pub struct CfRule {
    /// Query ref of the formatted column or measure (`selector.metadata`);
    /// `None`: every value field (charts).
    pub target: Option<String>,
    pub apply: ApplyTo,
    /// Inputs, referred to by index from the action.
    pub inputs: Vec<Input>,
    pub action: Action,
    /// Where the rule was read (`values.backColor`), for diagnostics.
    pub origin: String,
}

/// What a rule formats.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Background(ColorRule),
    FontColor(ColorRule),
    /// Chart data point fill.
    Fill(ColorRule),
    DataBars(DataBars),
    Icon(IconRule),
}

/// A conditional color.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorRule {
    /// Color scale over input 0.
    Gradient(Box<Gradient>),
    /// Rules: the first matching case's color.
    Rules(Cases<Json>),
    /// Input 0 is a CSS color.
    FieldValue,
}

/// A color scale (`linearGradient2` / `linearGradient3`).
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    pub min: Stop,
    pub mid: Option<Stop>,
    pub max: Stop,
    pub nulls: NullColor,
}

/// A gradient stop: its color (PBIR color expression) and an explicit
/// value (`None`: from the data).
#[derive(Debug, Clone, PartialEq)]
pub struct Stop {
    pub color: Json,
    pub value: Option<f64>,
}

/// How a gradient colors BLANK (`nullColoringStrategy`).
#[derive(Debug, Clone, PartialEq)]
pub enum NullColor {
    AsZero,
    NoColor,
    Color(Json),
}

/// Cases of a rule set: `(condition, value)` in order, and the default.
#[derive(Debug, Clone, PartialEq)]
pub struct Cases<T> {
    pub cases: Vec<(Cond, T)>,
    pub default: Option<T>,
}

/// A rule condition.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    Compare {
        kind: ComparisonKind,
        left: Operand,
        right: Operand,
    },
    And(Box<Cond>, Box<Cond>),
    Or(Box<Cond>, Box<Cond>),
    Not(Box<Cond>),
}

/// An arithmetic operator (`Arithmetic.Operator` 0–3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

/// A rule operand.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// The value of input `i` at the cell.
    Input(usize),
    Literal(LiteralValue),
    /// Smallest value of input `i` over the formatted cells.
    Min(usize),
    /// Largest value of input `i` over the formatted cells.
    Max(usize),
    /// Percentile `k` (0–1) of input `i` over the formatted cells.
    Percentile {
        input: usize,
        k: f64,
        exclusive: bool,
    },
    Arith {
        op: ArithOp,
        left: Box<Operand>,
        right: Box<Operand>,
    },
}

/// Data bars (on the cell's own value, input 0).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DataBars {
    pub positive: Option<Json>,
    pub negative: Option<Json>,
    pub axis: Option<Json>,
    pub reverse: bool,
    /// "Show bar only".
    pub hide_text: bool,
    pub min: Option<f64>,
    pub max: Option<f64>,
}

/// Where an icon goes relative to the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IconLayout {
    #[default]
    Before,
    After,
    IconOnly,
}

/// Vertical alignment of an icon in its cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum IconAlign {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// Conditional icons.
#[derive(Debug, Clone, PartialEq)]
pub struct IconRule {
    pub source: IconSource,
    pub layout: IconLayout,
    pub align: IconAlign,
}

/// Where an icon name comes from.
#[derive(Debug, Clone, PartialEq)]
pub enum IconSource {
    /// The first matching case's icon name.
    Rules(Cases<String>),
    /// Input 0 is an icon name.
    FieldValue,
}

/// The conditional formats of a visual.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CfSpec {
    pub rules: Vec<CfRule>,
    /// Conditional formatting that is not applied, with the reason.
    pub diagnostics: Vec<String>,
}

impl CfSpec {
    /// Reads the conditional formats of `visual` (see the module docs).
    pub fn parse(visual: &Visual) -> Self {
        parse::parse(visual)
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The distinct model fields the rules read.
    pub fn field_inputs(&self) -> Vec<&FieldExpr> {
        let mut out: Vec<&FieldExpr> = Vec::new();
        for rule in &self.rules {
            for input in &rule.inputs {
                if let Input::Field(f) = input
                    && !out.iter().any(|o| same_input(o, f))
                {
                    out.push(f);
                }
            }
        }
        out
    }

    /// Rules formatting the column or measure with `query_ref` (rules
    /// without a target included when `wildcard`).
    pub fn rules_for<'a>(&'a self, query_ref: &'a str, wildcard: bool) -> Vec<&'a CfRule> {
        self.rules
            .iter()
            .filter(|r| match &r.target {
                Some(t) => t == query_ref,
                None => wildcard,
            })
            .collect()
    }
}

/// `true` if two inputs are the same field (names case-insensitive).
fn same_input(a: &FieldExpr, b: &FieldExpr) -> bool {
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
        ) => fa == fb && same_input(ea, eb),
        (FieldExpr::Aggregation { .. }, _) | (_, FieldExpr::Aggregation { .. }) => false,
        _ => {
            std::mem::discriminant(a) == std::mem::discriminant(b)
                && match (a.entity(), a.property(), b.entity(), b.property()) {
                    (Some(t1), Some(p1), Some(t2), Some(p2)) => {
                        names_eq(t1, t2) && names_eq(p1, p2)
                    }
                    _ => a == b,
                }
        }
    }
}

/// The query ref a hidden field gets for `field` (also how renderers find
/// it among the hidden fields).
pub fn input_key(field: &FieldExpr) -> String {
    field.query_ref().unwrap_or_else(|| format!("{field:?}"))
}

/// `true` if a visible value column (`meta`) already holds `field`'s
/// values: the same measure (measure names are unique in a model), or the
/// same aggregation of the same column.
pub fn meta_matches(meta: &ColumnMeta, field: &FieldExpr) -> bool {
    let src = &meta.source;
    let prop = |p: &str| src.property.as_deref().is_some_and(|x| names_eq(x, p));
    match field {
        FieldExpr::Measure { property, .. } => {
            meta.is_measure && src.kind == FieldRefKind::Measure && prop(property)
        }
        FieldExpr::Aggregation { expr, function } => {
            let (Some(entity), Some(property)) = (expr.entity(), expr.property()) else {
                return false;
            };
            meta.is_measure
                && src.kind == FieldRefKind::Aggregation
                && src.aggregation == Some(*function)
                && src.entity.as_deref().is_some_and(|e| names_eq(e, entity))
                && prop(property)
        }
        _ => false,
    }
}

/// Finds the metadata index holding `field`'s values among `metas`: a
/// hidden field with its key first, then a visible value column
/// ([`meta_matches`]).
pub fn find_input(metas: &[ColumnMeta], field: &FieldExpr) -> Option<usize> {
    let key = input_key(field);
    metas
        .iter()
        .position(|m| m.hidden && m.query_ref == key)
        .or_else(|| {
            metas
                .iter()
                .position(|m| !m.hidden && meta_matches(m, field))
        })
}

/// A data bar inside a cell: fractions (0–1) of the bar area.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub from: f64,
    pub to: f64,
    pub paint: Paint,
    /// Axis position and color, when the range crosses zero.
    pub axis: Option<(f64, Color)>,
}

/// The conditional formatting of one cell.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CellStyle {
    pub background: Option<Paint>,
    pub font: Option<Color>,
    /// Chart data point fill.
    pub fill: Option<Paint>,
    pub bar: Option<Bar>,
    pub icon: Option<(Icon, IconLayout, IconAlign)>,
    /// Data bars with "show bar only".
    pub hide_text: bool,
}

impl CellStyle {
    pub fn is_empty(&self) -> bool {
        *self == CellStyle::default()
    }
}

/// The cells of one formatted column (a table column, a matrix value
/// field, a chart measure) as rules see them.
pub trait CellSource {
    /// Number of cells.
    fn cell_count(&self) -> usize;
    /// `true` for total and subtotal cells.
    fn is_total(&self, cell: usize) -> bool;
    /// Binds an input to a data column id (`None`: not in the data, the
    /// rule is skipped).
    fn bind(&self, input: &Input) -> Option<usize>;
    /// The value of bound column `column` at `cell`.
    fn value(&self, column: usize, cell: usize) -> Value;
}

/// Evaluates `rules` over every cell of `source` (later rules override
/// earlier ones for the same property).
pub fn evaluate_cells(
    rules: &[&CfRule],
    source: &dyn CellSource,
    theme: &ThemeResolver,
) -> Vec<CellStyle> {
    let n = source.cell_count();
    let mut out = vec![CellStyle::default(); n];
    for rule in rules {
        let Some(columns) = rule
            .inputs
            .iter()
            .map(|i| source.bind(i))
            .collect::<Option<Vec<usize>>>()
        else {
            continue;
        };
        let stats = Stats::collect(&columns, source, rule);
        let mut values = Vec::with_capacity(columns.len());
        for (cell, style) in out.iter_mut().enumerate() {
            let total = source.is_total(cell);
            let wanted = if total {
                rule.apply.totals()
            } else {
                rule.apply.values()
            };
            if !wanted {
                continue;
            }
            values.clear();
            values.extend(columns.iter().map(|&c| source.value(c, cell)));
            match &rule.action {
                Action::Background(r) => {
                    if let Some(p) = r.paint(&values, &stats, theme) {
                        style.background = Some(p);
                    }
                }
                Action::FontColor(r) => {
                    if let Some(p) = r.paint(&values, &stats, theme) {
                        style.font = Some(p.color);
                    }
                }
                Action::Fill(r) => {
                    if let Some(p) = r.paint(&values, &stats, theme) {
                        style.fill = Some(p);
                    }
                }
                Action::DataBars(b) if !total => {
                    style.hide_text |= b.hide_text;
                    if let Some(v) = values.first()
                        && let Some(bar) = b.bar(v, &stats, theme)
                    {
                        style.bar = Some(bar);
                    }
                }
                Action::DataBars(_) => {}
                Action::Icon(i) => {
                    if let Some(icon) = i.icon(&values, &stats) {
                        style.icon = Some((icon, i.layout, i.align));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests;
