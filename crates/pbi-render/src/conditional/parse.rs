//! PBIR `objects` → [`CfSpec`] (see the module docs of
//! [`crate::conditional`]).

use pbi_model::{AggregationFunction, ComparisonKind, FieldExpr, LiteralValue, Visual};
use serde_json::Value as Json;

use crate::theme::{as_bool, as_number, as_text};
use crate::visuals::charts::ChartKind;

use super::{
    Action, ApplyTo, ArithOp, Cases, CfRule, CfSpec, ColorRule, Cond, DataBars, Gradient,
    IconAlign, IconLayout, IconRule, IconSource, Input, NullColor, Operand, Stop, same_input,
};

/// Deepest expression nesting read (untrusted JSON).
const MAX_DEPTH: u8 = 24;

/// How a visual type takes conditional formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Grid,
    Chart,
    Other,
}

fn kind_of(visual_type: &str) -> Kind {
    match visual_type {
        "tableEx" | "table" | "pivotTable" | "matrix" => Kind::Grid,
        t => match ChartKind::from_visual_type(t) {
            Some(
                ChartKind::Bar(_)
                | ChartKind::Column(_)
                | ChartKind::Pie
                | ChartKind::Donut
                | ChartKind::Treemap
                | ChartKind::Scatter
                | ChartKind::Funnel,
            ) => Kind::Chart,
            _ => Kind::Other,
        },
    }
}

/// `true` for the `(object, property)` pairs a visual kind reads as
/// conditional formats.
fn handled(kind: Kind, object: &str, prop: &str) -> bool {
    match kind {
        Kind::Grid => {
            matches!(object, "values" | "columnFormatting")
                && matches!(prop, "backColor" | "fontColor" | "dataBars" | "icon")
        }
        Kind::Chart => object == "dataPoint" && prop == "fill",
        Kind::Other => false,
    }
}

fn unwrap_expr(v: &Json) -> &Json {
    v.get("expr").unwrap_or(v)
}

/// The expression of a conditional color property (`{"solid": {"color":
/// {"expr": …}}}`), `None` for static colors.
pub(super) fn cf_color_expr(v: &Json) -> Option<&Json> {
    let color = v
        .get("solid")
        .and_then(|s| s.get("color"))
        .map(unwrap_expr)
        .or_else(|| v.get("expr"))?;
    [
        "FillRule",
        "Conditional",
        "Measure",
        "Aggregation",
        "Column",
        "HierarchyLevel",
    ]
    .iter()
    .any(|k| color.get(*k).is_some())
    .then_some(color)
}

pub(super) fn parse(visual: &Visual) -> CfSpec {
    let mut spec = CfSpec::default();
    let Some(objects) = visual.objects.as_object() else {
        return spec;
    };
    let kind = kind_of(&visual.visual_type);
    for (object, entries) in objects {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let Some(props) = entry.get("properties").and_then(Json::as_object) else {
                continue;
            };
            let selector = entry.get("selector");
            for (prop, value) in props {
                if handled(kind, object, prop) {
                    let origin = format!("{object}.{prop}");
                    match read_rule(kind, prop, value, entry, selector, &origin) {
                        Ok(Some(rule)) => spec.rules.push(rule),
                        Ok(None) => {}
                        Err(reason) => spec.diagnostics.push(format!(
                            "conditional formatting not applied ({origin}{}): {reason}",
                            target_note(selector)
                        )),
                    }
                } else if cf_color_expr(value).is_some() {
                    spec.diagnostics.push(format!(
                        "conditional formatting not applied ({object}.{prop}): not supported for {}",
                        if visual.visual_type.is_empty() {
                            "this visual"
                        } else {
                            &visual.visual_type
                        }
                    ));
                }
            }
        }
    }
    if kind == Kind::Chart && !spec.rules.is_empty() && has_series(visual) {
        spec.rules.clear();
        spec.diagnostics.push(
            "conditional formatting not applied (dataPoint.fill): charts with a legend field \
             keep their series colors"
                .into(),
        );
    }
    spec
}

fn target_note(selector: Option<&Json>) -> String {
    selector
        .and_then(|s| s.get("metadata"))
        .and_then(Json::as_str)
        .map(|m| format!(" for {m}"))
        .unwrap_or_default()
}

fn has_series(visual: &Visual) -> bool {
    visual.projections.iter().any(|p| {
        matches!(
            p.role.as_str(),
            "Series" | "Legend" | "ColorBy" | "Breakdown"
        )
    })
}

/// `selector.data` → which cells; `Err` for per-instance (`scopeId`)
/// selectors.
fn apply_to(selector: Option<&Json>) -> Result<ApplyTo, String> {
    let Some(data) = selector
        .and_then(|s| s.get("data"))
        .and_then(Json::as_array)
    else {
        return Ok(ApplyTo::Values);
    };
    let mut apply = ApplyTo::Values;
    for d in data {
        if d.get("scopeId").is_some() {
            return Err("selectors of single data points are not supported".into());
        }
        if let Some(w) = d.get("dataViewWildcard") {
            apply = match w
                .get("matchingOption")
                .and_then(as_number)
                .map(|n| n as i64)
            {
                Some(0) => ApplyTo::Both,
                Some(2) => ApplyTo::Totals,
                _ => ApplyTo::Values,
            };
        } else if d.get("total").is_some() {
            apply = ApplyTo::Totals;
        }
    }
    Ok(apply)
}

/// One conditional property (`Ok(None)`: a static value).
fn read_rule(
    kind: Kind,
    prop: &str,
    value: &Json,
    entry: &Json,
    selector: Option<&Json>,
    origin: &str,
) -> Result<Option<CfRule>, String> {
    let target = selector
        .and_then(|s| s.get("metadata"))
        .and_then(Json::as_str)
        .map(str::to_owned);
    let mut inputs = Vec::new();
    let action = match prop {
        "backColor" | "fontColor" | "fill" => {
            let Some(expr) = cf_color_expr(value) else {
                return Ok(None);
            };
            let rule = color_rule(expr, &mut inputs)?;
            match prop {
                "backColor" => Action::Background(rule),
                "fontColor" => Action::FontColor(rule),
                _ => Action::Fill(rule),
            }
        }
        "dataBars" => {
            inputs.push(Input::Own);
            Action::DataBars(data_bars(value))
        }
        "icon" => {
            let props = entry.get("properties").unwrap_or(&Json::Null);
            Action::Icon(icon_rule(value, props, &mut inputs)?)
        }
        _ => return Ok(None),
    };
    let apply = apply_to(selector)?;
    if kind == Kind::Grid && target.is_none() {
        return Err("no column selected (selector.metadata)".into());
    }
    Ok(Some(CfRule {
        target,
        apply,
        inputs,
        action,
        origin: origin.to_owned(),
    }))
}

/// A model field: `Column`, `Measure`, `Aggregation` or `HierarchyLevel`
/// over a `SourceRef` with an `Entity`.
pub(super) fn field(e: &Json) -> Option<FieldExpr> {
    field_depth(e, 0)
}

fn field_depth(e: &Json, depth: u8) -> Option<FieldExpr> {
    if depth > MAX_DEPTH {
        return None;
    }
    let e = unwrap_expr(e);
    let entity = |x: &Json| {
        x.get("Expression")?
            .get("SourceRef")?
            .get("Entity")?
            .as_str()
            .map(str::to_owned)
    };
    let property = |x: &Json| x.get("Property")?.as_str().map(str::to_owned);
    if let Some(c) = e.get("Column") {
        return Some(FieldExpr::column(entity(c)?, property(c)?));
    }
    if let Some(m) = e.get("Measure") {
        return Some(FieldExpr::measure(entity(m)?, property(m)?));
    }
    if let Some(a) = e.get("Aggregation") {
        let inner = field_depth(a.get("Expression")?, depth + 1)?;
        let code = a.get("Function").and_then(as_number)?;
        return Some(FieldExpr::aggregation(
            inner,
            AggregationFunction::from_code(code as i64),
        ));
    }
    if let Some(h) = e.get("HierarchyLevel") {
        let level = h.get("Level")?.as_str()?.to_owned();
        let hier = h.get("Expression")?.get("Hierarchy")?;
        return Some(FieldExpr::HierarchyLevel {
            entity: entity(hier)?,
            hierarchy: hier.get("Hierarchy")?.as_str()?.to_owned(),
            level,
            variation: None,
        });
    }
    None
}

/// Index of `field` in `inputs`, adding it when new.
fn intern(inputs: &mut Vec<Input>, field: FieldExpr) -> usize {
    if let Some(i) = inputs
        .iter()
        .position(|x| matches!(x, Input::Field(f) if same_input(f, &field)))
    {
        return i;
    }
    inputs.push(Input::Field(field));
    inputs.len() - 1
}

fn color_rule(e: &Json, inputs: &mut Vec<Input>) -> Result<ColorRule, String> {
    if let Some(fr) = e.get("FillRule") {
        let input = fr
            .get("Input")
            .and_then(field)
            .ok_or("the gradient's input is not a model field")?;
        intern(inputs, input);
        let rule = fr.get("FillRule").ok_or("the gradient has no FillRule")?;
        let (g, three) = match (rule.get("linearGradient2"), rule.get("linearGradient3")) {
            (Some(g), _) => (g, false),
            (None, Some(g)) => (g, true),
            _ => {
                let name = rule
                    .as_object()
                    .and_then(|m| m.keys().next().cloned())
                    .unwrap_or_default();
                return Err(format!("fill rule `{name}` is not supported"));
            }
        };
        let stop = |name: &str| -> Result<Stop, String> {
            let s = g
                .get(name)
                .ok_or_else(|| format!("the gradient has no `{name}` color"))?;
            Ok(Stop {
                color: s
                    .get("color")
                    .cloned()
                    .ok_or_else(|| format!("the gradient's `{name}` has no color"))?,
                value: s.get("value").and_then(as_number),
            })
        };
        return Ok(ColorRule::Gradient(Box::new(Gradient {
            min: stop("min")?,
            mid: if three { Some(stop("mid")?) } else { None },
            max: stop("max")?,
            nulls: null_color(g.get("nullColoringStrategy")),
        })));
    }
    if let Some(c) = e.get("Conditional") {
        return Ok(ColorRule::Rules(cases(c, inputs, |v| Some(v.clone()))?));
    }
    if let Some(f) = field(e) {
        intern(inputs, f);
        return Ok(ColorRule::FieldValue);
    }
    Err("unsupported color expression".into())
}

fn null_color(v: Option<&Json>) -> NullColor {
    let Some(v) = v else {
        return NullColor::AsZero;
    };
    let strategy = v
        .get("strategy")
        .and_then(as_text)
        .unwrap_or_default()
        .to_ascii_lowercase();
    match strategy.as_str() {
        "nocolor" | "dontcolor" | "none" | "blank" | "asblank" => NullColor::NoColor,
        "specificcolor" | "color" => v
            .get("color")
            .cloned()
            .map_or(NullColor::AsZero, NullColor::Color),
        _ => NullColor::AsZero,
    }
}

fn cases<T>(
    c: &Json,
    inputs: &mut Vec<Input>,
    value: impl Fn(&Json) -> Option<T>,
) -> Result<Cases<T>, String> {
    let list = c
        .get("Cases")
        .and_then(Json::as_array)
        .ok_or("rules without cases")?;
    let mut out = Vec::with_capacity(list.len());
    for case in list.iter().take(256) {
        let condition = case.get("Condition").ok_or("a rule has no condition")?;
        let v = case.get("Value").ok_or("a rule has no value")?;
        out.push((
            cond(condition, inputs, 0)?,
            value(v).ok_or("a rule's value is not supported")?,
        ));
    }
    Ok(Cases {
        cases: out,
        default: c.get("DefaultValue").and_then(&value),
    })
}

fn cond(e: &Json, inputs: &mut Vec<Input>, depth: u8) -> Result<Cond, String> {
    if depth > MAX_DEPTH {
        return Err("the rule is nested too deeply".into());
    }
    let e = unwrap_expr(e);
    if let Some(c) = e.get("Comparison") {
        let kind = c
            .get("ComparisonKind")
            .and_then(as_number)
            .and_then(|k| ComparisonKind::from_code(k as i64))
            .ok_or("unknown comparison")?;
        let left = operand(
            c.get("Left").ok_or("comparison without Left")?,
            inputs,
            depth,
        )?;
        let right = operand(
            c.get("Right").ok_or("comparison without Right")?,
            inputs,
            depth,
        )?;
        return Ok(Cond::Compare { kind, left, right });
    }
    for (key, and) in [("And", true), ("Or", false)] {
        if let Some(b) = e.get(key) {
            let l = cond(b.get("Left").ok_or("missing Left")?, inputs, depth + 1)?;
            let r = cond(b.get("Right").ok_or("missing Right")?, inputs, depth + 1)?;
            return Ok(if and {
                Cond::And(Box::new(l), Box::new(r))
            } else {
                Cond::Or(Box::new(l), Box::new(r))
            });
        }
    }
    if let Some(n) = e.get("Not") {
        let inner = cond(
            n.get("Expression").ok_or("Not without Expression")?,
            inputs,
            depth + 1,
        )?;
        return Ok(Cond::Not(Box::new(inner)));
    }
    let name = e
        .as_object()
        .and_then(|m| m.keys().next().cloned())
        .unwrap_or_default();
    Err(format!("rule condition `{name}` is not supported"))
}

fn operand(e: &Json, inputs: &mut Vec<Input>, depth: u8) -> Result<Operand, String> {
    if depth > MAX_DEPTH {
        return Err("the rule is nested too deeply".into());
    }
    let e = unwrap_expr(e);
    if let Some(raw) = e
        .get("Literal")
        .and_then(|l| l.get("Value"))
        .and_then(Json::as_str)
    {
        return LiteralValue::parse(raw)
            .map(Operand::Literal)
            .map_err(|_| format!("literal {raw} is not supported"));
    }
    if let Some(f) = field(e) {
        return Ok(Operand::Input(intern(inputs, f)));
    }
    if let Some(s) = e.get("ScopedEval") {
        let scoped = s
            .get("Scope")
            .and_then(Json::as_array)
            .is_none_or(Vec::is_empty);
        if !scoped {
            return Err("ScopedEval with a scope is not supported".into());
        }
        let inner = s.get("Expression").ok_or("ScopedEval without Expression")?;
        return visual_aggregate(unwrap_expr(inner), inputs)
            .ok_or_else(|| "ScopedEval of this expression is not supported".into());
    }
    if let Some(op) = visual_aggregate(e, inputs) {
        return Ok(op);
    }
    if let Some(a) = e.get("Arithmetic") {
        let op = match a.get("Operator").and_then(as_number).map(|n| n as i64) {
            Some(0) => ArithOp::Add,
            Some(1) => ArithOp::Subtract,
            Some(2) => ArithOp::Multiply,
            Some(3) => ArithOp::Divide,
            _ => return Err("unknown arithmetic operator".into()),
        };
        let left = operand(a.get("Left").ok_or("missing Left")?, inputs, depth + 1)?;
        let right = operand(a.get("Right").ok_or("missing Right")?, inputs, depth + 1)?;
        return Ok(Operand::Arith {
            op,
            left: Box::new(left),
            right: Box::new(right),
        });
    }
    let name = e
        .as_object()
        .and_then(|m| m.keys().next().cloned())
        .unwrap_or_default();
    Err(format!("rule operand `{name}` is not supported"))
}

/// `Min`, `Max` or `Percentile` of a field over the visual's data.
fn visual_aggregate(e: &Json, inputs: &mut Vec<Input>) -> Option<Operand> {
    if let Some(m) = e.get("Min") {
        return Some(Operand::Min(intern(inputs, field(m.get("Expression")?)?)));
    }
    if let Some(m) = e.get("Max") {
        return Some(Operand::Max(intern(inputs, field(m.get("Expression")?)?)));
    }
    if let Some(p) = e.get("Percentile") {
        let k = p.get("K").and_then(as_number)?;
        let exclusive = p.get("Exclusive").and_then(as_bool).unwrap_or(false);
        let input = intern(inputs, field(p.get("Expression")?)?);
        return Some(Operand::Percentile {
            input,
            k,
            exclusive,
        });
    }
    None
}

fn data_bars(v: &Json) -> DataBars {
    let first = |names: &[&str]| names.iter().find_map(|n| v.get(*n));
    DataBars {
        positive: first(&["positiveColor", "positiveFill"]).cloned(),
        negative: first(&["negativeColor", "negativeFill"]).cloned(),
        axis: first(&["axisColor"]).cloned(),
        reverse: first(&["reverseDirection"])
            .and_then(as_bool)
            .unwrap_or(false),
        hide_text: first(&["hideText", "showBarOnly"])
            .and_then(as_bool)
            .unwrap_or(false),
        min: first(&["minValue", "min", "minimum"]).and_then(as_number),
        max: first(&["maxValue", "max", "maximum"]).and_then(as_number),
    }
}

fn icon_rule(v: &Json, props: &Json, inputs: &mut Vec<Input>) -> Result<IconRule, String> {
    let source = v
        .get("value")
        .map(unwrap_expr)
        .or_else(|| v.get("expr"))
        .unwrap_or(v);
    let layout = v
        .get("layout")
        .or_else(|| props.get("iconLayout"))
        .and_then(as_text)
        .map(|t| t.to_ascii_lowercase());
    let layout = match layout.as_deref() {
        Some("after" | "right" | "rightofdata" | "afterdata") => IconLayout::After,
        Some("icononly" | "only" | "iconsonly") => IconLayout::IconOnly,
        _ => IconLayout::Before,
    };
    let align = v
        .get("verticalAlignment")
        .or_else(|| props.get("iconVerticalAlignment"))
        .and_then(as_text)
        .map(|t| t.to_ascii_lowercase());
    let align = match align.as_deref() {
        Some("middle" | "center" | "centre") => IconAlign::Middle,
        Some("bottom") => IconAlign::Bottom,
        _ => IconAlign::Top,
    };
    let source = if let Some(c) = source.get("Conditional") {
        IconSource::Rules(cases(c, inputs, as_text)?)
    } else if let Some(f) = field(source) {
        intern(inputs, f);
        IconSource::FieldValue
    } else {
        return Err("unsupported icon expression".into());
    };
    Ok(IconRule {
        source,
        layout,
        align,
    })
}
