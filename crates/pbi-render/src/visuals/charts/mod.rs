//! Charts: pie/donut, clustered/stacked/100% bar & column, line/area/
//! stacked area and line + column combos, scatter/bubble, funnel, gauge,
//! treemap and waterfall, with axes, gridlines, legends, data labels,
//! category scrolling and hit ids. (owner: visuals-charts)
//!
//! Charts draw only the plot inside `frame.content`; the container chrome
//! (title, background, border) comes from the frame.
//!
//! # Data
//!
//! [`ChartData::from_dataview`] reads a categorical [`DataView`]: category
//! labels (several category columns are joined with a space; BLANK shows
//! as `(Blank)`), and one [`SeriesData`] per value column of the `Y` /
//! `Values` roles (`Y2` for the line part of combos; `Tooltips` and other
//! roles are ignored). Non-numeric, BLANK, NaN and ±∞ values are gaps.
//!
//! # Formatting objects
//!
//! | Object | Properties |
//! |---|---|
//! | `legend` | `show`, `position` (`Top`, `Bottom`, `Left`, `Right`, `TopCenter`, `BottomCenter`, `LeftCenter`, `RightCenter`), `showTitle`, `titleText`, `labelColor`, `fontSize`, `fontFamily` |
//! | `categoryAxis` | `show`, `showAxisTitle`, `titleText`, `titleColor`, `titleFontSize`, `labelColor`, `fontSize`, `fontFamily`, `gridlineShow`, `axisType` (`Categorical`/`Continuous`), `innerPadding`, `preferredCategoryWidth`, `maxMarginFactor` |
//! | `valueAxis` (`y2Axis` for combo lines) | `show`, `showAxisTitle`, `titleText`, `start`, `end`, `labelDisplayUnits`, `labelPrecision`, `gridlineShow`, `gridlineColor`, `gridlineThickness`, `gridlineStyle` (`dotted`, `dashed`, `solid`), fonts and colors as above |
//! | `dataPoint` | `fill` per category / series value (`selector.data` scope ids) or measure (`selector.metadata`), `defaultColor` |
//! | `labels` | `show`, `color`, `fontSize`, `fontFamily`, `labelDisplayUnits`, `labelPrecision`, `labelStyle` (pie: `Category`, `Data value`, `Percent of total`, combinations, `All detail labels`), `labelPosition` (pie `outside`/`inside`/`preferOutside`; bars `OutsideEnd`/`InsideEnd`/`InsideCenter`/`InsideBase`/`Auto`) |
//! | `slices` | `innerRadiusRatio` (donut, % of the radius, default 50) |
//! | `lineStyles` | `strokeWidth`, `showMarker`, `markerSize`, `lineStyle` |
//!
//! Scatter, funnel, gauge, treemap and waterfall read their own objects
//! too (`categoryLabels`, `bubbles`, `fillPoint`, `percentBarLabel`,
//! `axis`, `calloutValue`, `targetLabel`, `sentimentColors`,
//! `breakdown`): see their modules.
//!
//! # Conditional colors and dynamic format strings
//!
//! Bar, column, pie, donut, treemap, scatter and funnel charts without a
//! legend field honour conditional `dataPoint.fill` (gradient, rules,
//! field value; [`crate::conditional`]) per category, over every value
//! field or the one in `selector.metadata`
//! ([`SeriesData::conditional_fill`]); it wins over explicit and palette
//! colors, and a point the rules leave uncolored keeps its usual color.
//! Data labels format each value with its cell's effective format string
//! ([`SeriesData::format_at`]: a calculation item's percent format shows
//! as a percent next to currency values); axes keep the field's format
//! string.
//!
//! # Hit ids
//!
//! Every slice, bar, line point, scatter marker, funnel bar, treemap
//! rectangle and waterfall column is a group with
//! [`HitId::data_point`]`(visual, series, category index)` (pie slices use
//! series 0); line points are invisible circles over the markers. `series`
//! indexes [`ChartData::series`], so a click or hover resolves to the
//! category row and series value; columns that stand for a category only
//! (waterfall pillars, "Other") use `series = ChartData::series.len()`.

pub mod axis;
pub mod bar;
pub mod cartesian;
pub mod colors;
pub mod format;
pub mod funnel;
pub mod gauge;
pub mod labels;
pub mod legend;
pub mod line;
pub mod pie;
pub mod scale;
pub mod scatter;
pub mod treemap;
pub mod waterfall;

use pbi_engine::Value;
use pbi_model::DataType;

use crate::conditional::{Action, CellSource, CfSpec, Input, evaluate_cells, find_input};
use crate::dataview::{BLANK_LABEL, Categorical, CellValue, ColumnMeta, DataView};
use crate::scene::{Color, Group, HitId, Paint, PathData, PathNode, Rect, Stroke, TextAnchor};
use crate::text::{FontBook, TextStyle};
use crate::theme::{PropLookup, ThemeResolver, VisualProps};

use super::VisualCtx;

/// Most categories read from a DataView (untrusted-input guard).
pub const MAX_CATEGORIES: usize = 100_000;
/// Most series read from a DataView.
pub const MAX_SERIES: usize = 1_000;
/// Opacity of data points that are not part of the current selection.
pub const DIMMED_OPACITY: f64 = 0.4;

/// Chart visual types handled by this module.
pub fn handles(visual_type: &str) -> bool {
    ChartKind::from_visual_type(visual_type).is_some()
}

/// Bar/column series arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stacking {
    Clustered,
    Stacked,
    /// 100% stacked.
    Percent,
}

/// The chart family of a visual type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChartKind {
    Pie,
    Donut,
    /// Horizontal bars.
    Bar(Stacking),
    /// Vertical columns.
    Column(Stacking),
    Line,
    Area,
    StackedArea,
    /// Columns (`Y`) + lines (`Y2`).
    Combo(Stacking),
    /// Scatter / bubble chart (`X`, `Y`, `Size`).
    Scatter,
    Funnel,
    /// Radial gauge (`Y`, `MinValue`, `MaxValue`, `TargetValue`).
    Gauge,
    Treemap,
    Waterfall,
}

impl ChartKind {
    /// Power BI `visualType` → kind (`barChart`/`columnChart` are the
    /// stacked variants, as in Power BI).
    pub fn from_visual_type(visual_type: &str) -> Option<Self> {
        use Stacking::{Clustered, Percent, Stacked};
        Some(match visual_type {
            "pieChart" => ChartKind::Pie,
            "donutChart" => ChartKind::Donut,
            "clusteredBarChart" => ChartKind::Bar(Clustered),
            "barChart" | "stackedBarChart" => ChartKind::Bar(Stacked),
            "hundredPercentStackedBarChart" => ChartKind::Bar(Percent),
            "clusteredColumnChart" => ChartKind::Column(Clustered),
            "columnChart" | "stackedColumnChart" => ChartKind::Column(Stacked),
            "hundredPercentStackedColumnChart" => ChartKind::Column(Percent),
            "lineChart" => ChartKind::Line,
            "areaChart" => ChartKind::Area,
            "stackedAreaChart" => ChartKind::StackedArea,
            "lineClusteredColumnComboChart" => ChartKind::Combo(Clustered),
            "lineStackedColumnComboChart" => ChartKind::Combo(Stacked),
            "scatterChart" => ChartKind::Scatter,
            "funnel" => ChartKind::Funnel,
            "gauge" => ChartKind::Gauge,
            "treemap" => ChartKind::Treemap,
            "waterfallChart" => ChartKind::Waterfall,
            _ => return None,
        })
    }
}

/// One plotted series (a value column, per series value).
#[derive(Debug, Clone, PartialEq)]
pub struct SeriesData {
    /// Legend text: the series value, or the measure's display name.
    pub name: String,
    /// Display name of the measure.
    pub measure_name: String,
    /// Measure query ref (`dataPoint` `metadata` selectors).
    pub query_ref: String,
    pub format_string: Option<String>,
    /// Value of the series (legend) field.
    pub series_value: Option<Value>,
    /// Display text of the series value (`(Blank)` for BLANK), without the
    /// measure name.
    pub series_label: Option<String>,
    /// Index of the source column in the DataView's `categorical.values`.
    pub column: usize,
    /// Bound to `Y2` (combo line / secondary axis).
    pub secondary: bool,
    /// One value per category (`None`: BLANK, text, NaN, ±∞).
    pub values: Vec<Option<f64>>,
    /// The DataView's formatted text per category.
    pub formatted: Vec<String>,
    /// Per category, the cell's own effective format string when it has
    /// one ([`CellValue::format`]; empty when no cell has).
    pub formats: Vec<Option<String>>,
    /// Conditional fill per category (empty without conditional colors).
    pub fills: Vec<Option<Color>>,
}

impl SeriesData {
    pub fn value(&self, i: usize) -> Option<f64> {
        self.values.get(i).copied().flatten()
    }

    pub fn formatted(&self, i: usize) -> &str {
        self.formatted.get(i).map_or("", String::as_str)
    }

    /// The effective format string of category `i`'s value: the cell's
    /// own (dynamic format strings), else the field's.
    pub fn format_at(&self, i: usize) -> Option<&str> {
        match self.formats.get(i).and_then(Option::as_deref) {
            Some("") => None,
            Some(f) => Some(f),
            None => self.format_string.as_deref(),
        }
    }

    /// The conditional fill of category `i`, if a rule colors it.
    pub fn conditional_fill(&self, i: usize) -> Option<Color> {
        self.fills.get(i).copied().flatten()
    }
}

/// The values of one series as conditional-formatting rules see them.
struct SeriesCells<'a> {
    dv: &'a DataView,
    cat: &'a Categorical,
    column: usize,
    len: usize,
}

const HIDDEN: usize = usize::MAX / 4;

impl CellSource for SeriesCells<'_> {
    fn cell_count(&self) -> usize {
        self.len
    }

    fn is_total(&self, _cell: usize) -> bool {
        false
    }

    fn bind(&self, input: &Input) -> Option<usize> {
        let Input::Field(field) = input else {
            return Some(self.column);
        };
        let meta = find_input(&self.dv.metadata, field)?;
        if let Some(c) = self
            .cat
            .values
            .iter()
            .position(|v| v.meta_idx == meta && v.series.is_none())
        {
            return Some(c);
        }
        let h = self.cat.hidden.iter().position(|h| h.meta_idx == meta)?;
        Some(HIDDEN + h)
    }

    fn value(&self, column: usize, cell: usize) -> Value {
        let found = if column >= HIDDEN {
            self.cat
                .hidden
                .get(column - HIDDEN)
                .and_then(|h| h.values.get(cell))
        } else {
            self.cat.values.get(column).and_then(|v| v.values.get(cell))
        };
        found.map(|c| c.raw.clone()).unwrap_or_default()
    }
}

/// Chart-ready view of a categorical DataView.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChartData {
    /// Category labels (`(Blank)` for BLANK).
    pub categories: Vec<String>,
    /// Raw values per category row, one entry per category column.
    pub category_values: Vec<Vec<Value>>,
    /// Metadata of the category columns.
    pub category_metas: Vec<ColumnMeta>,
    pub series: Vec<SeriesData>,
    /// Metadata of the series (legend) field.
    pub series_meta: Option<ColumnMeta>,
    /// The query cut rows (data-reduced hint).
    pub truncated: bool,
}

fn cell_label(c: Option<&CellValue>) -> String {
    match c {
        Some(c) if !c.formatted.is_empty() => c.formatted.clone(),
        Some(c) if !c.is_blank() => String::new(),
        _ => BLANK_LABEL.to_owned(),
    }
}

impl ChartData {
    /// Reads `dv` (an empty view when it is not categorical).
    pub fn from_dataview(dv: &DataView) -> Self {
        let Some(cat) = dv.categorical.as_ref() else {
            return Self {
                truncated: dv.truncated,
                ..Self::default()
            };
        };
        let n = cat.len().min(MAX_CATEGORIES);
        let category_metas: Vec<ColumnMeta> = cat
            .categories
            .iter()
            .filter_map(|c| dv.meta(c.meta_idx).cloned())
            .collect();
        let (categories, category_values) = if cat.categories.is_empty() {
            (vec![String::new(); n], vec![Vec::new(); n])
        } else {
            (0..n)
                .map(|i| {
                    let labels: Vec<String> = cat
                        .categories
                        .iter()
                        .map(|c| cell_label(c.values.get(i)))
                        .filter(|s| !s.is_empty())
                        .collect();
                    let raws = cat
                        .categories
                        .iter()
                        .map(|c| c.values.get(i).map_or(Value::Blank, |v| v.raw.clone()))
                        .collect();
                    (labels.join(" "), raws)
                })
                .unzip()
        };
        let has_role = |idx: usize, roles: &[&str]| {
            dv.meta(idx)
                .is_some_and(|m| m.roles.iter().any(|r| roles.contains(&r.as_str())))
        };
        let primary = ["Y", "Values"];
        let secondary = ["Y2"];
        let any_known = cat
            .values
            .iter()
            .any(|v| has_role(v.meta_idx, &primary) || has_role(v.meta_idx, &secondary));
        let multi_measure = {
            let mut idx: Vec<usize> = cat.values.iter().map(|v| v.meta_idx).collect();
            idx.sort_unstable();
            idx.dedup();
            idx.len() > 1
        };
        let mut series = Vec::new();
        for (column, vc) in cat.values.iter().enumerate().take(MAX_SERIES) {
            let is_secondary =
                has_role(vc.meta_idx, &secondary) && !has_role(vc.meta_idx, &primary);
            let wanted = if any_known {
                has_role(vc.meta_idx, &primary) || is_secondary
            } else {
                !has_role(vc.meta_idx, &["Tooltips"])
            };
            if !wanted {
                continue;
            }
            let Some(meta) = dv.meta(vc.meta_idx) else {
                continue;
            };
            let name = match &vc.series {
                Some(s) if multi_measure => {
                    format!("{} {}", cell_label(Some(s)), meta.display_name)
                }
                Some(s) => cell_label(Some(s)),
                None => meta.display_name.clone(),
            };
            let values = (0..n)
                .map(|i| {
                    vc.values
                        .get(i)
                        .filter(|c| c.raw.is_numeric())
                        .and_then(|c| c.raw.as_f64())
                        .filter(|v| v.is_finite())
                })
                .collect();
            let formatted = (0..n)
                .map(|i| {
                    vc.values
                        .get(i)
                        .map(|c| c.formatted.clone())
                        .unwrap_or_default()
                })
                .collect();
            let formats = if vc.values.iter().any(|c| c.format.is_some()) {
                (0..n)
                    .map(|i| vc.values.get(i).and_then(|c| c.format.clone()))
                    .collect()
            } else {
                Vec::new()
            };
            series.push(SeriesData {
                name,
                measure_name: meta.display_name.clone(),
                query_ref: meta.query_ref.clone(),
                format_string: meta.format_string.clone(),
                series_value: vc.series.as_ref().map(|s| s.raw.clone()),
                series_label: vc.series.as_ref().map(|s| cell_label(Some(s))),
                column,
                secondary: is_secondary,
                values,
                formatted,
                formats,
                fills: Vec::new(),
            });
        }
        Self {
            categories,
            category_values,
            category_metas,
            series,
            series_meta: cat.series_meta_idx.and_then(|i| dv.meta(i).cloned()),
            truncated: dv.truncated,
        }
    }

    /// Evaluates the visual's conditional `dataPoint.fill` rules into
    /// [`SeriesData::fills`] (see the module docs).
    pub fn apply_conditional(
        &mut self,
        visual: &pbi_model::Visual,
        dv: &DataView,
        theme: &ThemeResolver,
    ) {
        let spec = CfSpec::parse(visual);
        if spec.is_empty() || self.series_meta.is_some() {
            return;
        }
        let Some(cat) = dv.categorical.as_ref() else {
            return;
        };
        let len = self.categories.len();
        for s in &mut self.series {
            let rules: Vec<_> = spec
                .rules_for(&s.query_ref, true)
                .into_iter()
                .filter(|r| matches!(r.action, Action::Fill(_)))
                .collect();
            if rules.is_empty() {
                continue;
            }
            let source = SeriesCells {
                dv,
                cat,
                column: s.column,
                len,
            };
            s.fills = evaluate_cells(&rules, &source, theme)
                .into_iter()
                .map(|style| style.fill.map(|p| p.color))
                .collect();
        }
    }

    pub fn len(&self) -> usize {
        self.categories.len()
    }

    /// No category or no series.
    pub fn is_empty(&self) -> bool {
        self.categories.is_empty() || self.series.is_empty()
    }

    /// `true` when no series has a value.
    pub fn all_blank(&self) -> bool {
        self.series
            .iter()
            .all(|s| s.values.iter().all(Option::is_none))
    }

    /// Category axis title: the category display names joined with "and".
    pub fn category_title(&self) -> Option<String> {
        let names: Vec<&str> = self
            .category_metas
            .iter()
            .map(|m| m.display_name.as_str())
            .collect();
        (!names.is_empty()).then(|| names.join(" and "))
    }

    /// Value axis title: the distinct measure names joined with "and".
    pub fn value_title(&self, secondary: bool) -> Option<String> {
        let mut names: Vec<&str> = Vec::new();
        for s in self.series.iter().filter(|s| s.secondary == secondary) {
            if !names.contains(&s.measure_name.as_str()) {
                names.push(&s.measure_name);
            }
        }
        (!names.is_empty()).then(|| names.join(" and "))
    }

    /// `(column, value)` pairs identifying category row `i` (selectors).
    pub fn category_fields(&self, i: usize) -> Vec<(&ColumnMeta, &Value)> {
        match self.category_values.get(i) {
            Some(vals) => self.category_metas.iter().zip(vals.iter()).collect(),
            None => Vec::new(),
        }
    }

    /// The single category column holds dates or numbers (candidate for a
    /// continuous axis).
    pub fn continuous_kind(&self) -> Option<ContinuousKind> {
        if self.category_metas.len() != 1 {
            return None;
        }
        let meta = self.category_metas.first()?;
        let first = self
            .category_values
            .iter()
            .filter_map(|v| v.first())
            .find(|v| !v.is_blank());
        match (&meta.data_type, first) {
            (Some(DataType::DateTime), _) | (_, Some(Value::DateTime(_))) => {
                Some(ContinuousKind::Date)
            }
            (Some(t), _) if t.is_numeric() => Some(ContinuousKind::Number),
            (_, Some(v)) if v.is_numeric() => Some(ContinuousKind::Number),
            _ => None,
        }
    }

    /// Numeric position of category `i` on a continuous axis (dates as OLE
    /// serial days).
    pub fn category_number(&self, i: usize) -> Option<f64> {
        self.category_values
            .get(i)?
            .first()?
            .as_f64()
            .filter(|v| v.is_finite())
    }
}

/// Kind of a continuous category axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuousKind {
    Date,
    Number,
}

/// Shared state of one chart render.
#[derive(Debug, Clone, Copy)]
pub struct ChartCtx<'a> {
    pub visual: &'a VisualCtx<'a>,
    pub props: VisualProps<'a>,
    pub data: &'a ChartData,
    pub kind: ChartKind,
    /// The drawing area (`frame.content`).
    pub area: Rect,
}

impl<'a> ChartCtx<'a> {
    pub fn objects(&self) -> PropLookup<'a> {
        self.props.objects()
    }

    pub fn resolver(&self) -> &'a ThemeResolver {
        self.visual.theme
    }

    pub fn fonts(&self) -> &'a FontBook {
        self.visual.fonts
    }

    /// The query result being drawn.
    pub fn dataview(&self) -> Option<&'a DataView> {
        match self.visual.data {
            Some(Ok(dv)) => Some(dv),
            _ => None,
        }
    }

    pub fn hit(&self, series: usize, index: usize) -> HitId {
        HitId::data_point(self.visual.visual.name.clone(), series, index)
    }

    pub fn data_point_colors(&self) -> colors::DataPointColors<'a> {
        colors::DataPointColors::new(&self.visual.visual.objects, self.visual.theme)
    }

    /// Per-category opacity when this visual is the source of the current
    /// point selection (selected points opaque, others dimmed).
    pub fn selection_opacity(&self) -> Option<Vec<f64>> {
        let sel = self.visual.state.selected_points.as_ref()?;
        if sel.source != self.visual.visual.name || sel.points.is_empty() {
            return None;
        }
        let out = (0..self.data.len())
            .map(|i| {
                let fields = self.data.category_fields(i);
                let hit = sel.points.iter().any(|point| {
                    point.iter().all(|pf| {
                        let prop = pf.field.property();
                        match fields
                            .iter()
                            .find(|(m, _)| m.source.property.as_deref() == prop)
                        {
                            Some((_, v)) => **v == pf.value,
                            // Series fields of the point do not dim categories.
                            None => true,
                        }
                    })
                });
                if hit { 1.0 } else { DIMMED_OPACITY }
            })
            .collect();
        Some(out)
    }
}

/// Renders a chart into `frame.content` (nothing for an unknown type, a
/// missing query result or a too small box).
pub fn render(ctx: &VisualCtx<'_>) -> Option<Group> {
    let kind = ChartKind::from_visual_type(&ctx.visual.visual_type)?;
    let area = ctx.frame.content;
    if area.is_empty() || area.width < 4.0 || area.height < 4.0 {
        return None;
    }
    let dv = match ctx.data {
        Some(Ok(dv)) => dv,
        _ => return None,
    };
    let mut data = ChartData::from_dataview(dv);
    data.apply_conditional(ctx.visual, dv, ctx.theme);
    let cc = ChartCtx {
        visual: ctx,
        props: ctx.theme.visual(ctx.visual),
        data: &data,
        kind,
        area,
    };
    let mut g = match kind {
        ChartKind::Pie | ChartKind::Donut => pie::render(&cc),
        ChartKind::Bar(stacking) => bar::render_bar(&cc, stacking),
        ChartKind::Column(stacking) => bar::render_column(&cc, stacking, false),
        ChartKind::Combo(stacking) => bar::render_column(&cc, stacking, true),
        ChartKind::Line | ChartKind::Area | ChartKind::StackedArea => line::render(&cc),
        ChartKind::Scatter => scatter::render(&cc),
        ChartKind::Funnel => funnel::render(&cc),
        ChartKind::Gauge => gauge::render(&cc),
        ChartKind::Treemap => treemap::render(&cc),
        ChartKind::Waterfall => waterfall::render(&cc),
    };
    if data.truncated {
        data_reduced_hint(&mut g, area, ctx.theme);
    }
    Some(g.with_class("pbi-chart"))
}

/// A small `i` badge in the top-right corner: not every data point is
/// shown (the query cut rows).
fn data_reduced_hint(g: &mut Group, area: Rect, theme: &ThemeResolver) {
    let color = theme
        .named_color("foregroundNeutralSecondary")
        .unwrap_or(Color::rgb(0x60, 0x5E, 0x5C));
    let r = 5.0;
    let (cx, cy) = (area.right() - r - 1.0, area.y + r + 1.0);
    let mut hint = Group::new().with_class("pbi-data-reduced");
    hint.push(
        PathNode::new(PathData::pie_slice(cx, cy, r, 0.0, 0.0, 360.0))
            .fill(Paint::solid(Color::WHITE))
            .stroke(Stroke::new(color, 1.0)),
    );
    let style = TextStyle::default()
        .with_size_pt(6.0)
        .with_color(color)
        .bold();
    let baseline = labels::baseline_centered(&style, cy);
    hint.text(cx, baseline, "i", style, TextAnchor::Middle);
    g.push(hint);
}
