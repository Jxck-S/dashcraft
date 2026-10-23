//! Rule evaluation: statistics over the formatted cells, conditions,
//! gradients, data bars and icons.

use pbi_engine::Value;
use pbi_model::{ComparisonKind, LiteralValue};

use crate::scene::{Color, Paint};
use crate::theme::ThemeResolver;

use super::color::{lerp, parse_css_color};
use super::{
    Action, ArithOp, Bar, Cases, CellSource, CfRule, ColorRule, Cond, DataBars, Gradient, Icon,
    IconRule, IconSource, NullColor, Operand,
};

/// Default negative data bar color.
const BAR_NEGATIVE: Color = Color::rgb(0xD6, 0x45, 0x54);
/// Default data bar axis color.
const BAR_AXIS: Color = Color::rgb(0x25, 0x24, 0x23);

/// Numeric statistics of one input over the formatted cells (totals
/// excluded).
#[derive(Debug, Clone, Default, PartialEq)]
struct InputStats {
    min: Option<f64>,
    max: Option<f64>,
    /// Sorted finite values (only when a percentile needs them).
    sorted: Vec<f64>,
}

/// Statistics of a rule's inputs over the formatted cells.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Stats {
    inputs: Vec<InputStats>,
}

fn number(v: &Value) -> Option<f64> {
    if !v.is_numeric() {
        return None;
    }
    v.as_f64().filter(|x| x.is_finite())
}

fn op_needs_sorted(op: &Operand, input: usize) -> bool {
    match op {
        Operand::Percentile { input: i, .. } => *i == input,
        Operand::Arith { left, right, .. } => {
            op_needs_sorted(left, input) || op_needs_sorted(right, input)
        }
        _ => false,
    }
}

fn cond_needs_sorted(c: &Cond, input: usize) -> bool {
    match c {
        Cond::Compare { left, right, .. } => {
            op_needs_sorted(left, input) || op_needs_sorted(right, input)
        }
        Cond::And(a, b) | Cond::Or(a, b) => {
            cond_needs_sorted(a, input) || cond_needs_sorted(b, input)
        }
        Cond::Not(a) => cond_needs_sorted(a, input),
    }
}

fn cases_need_sorted<T>(cases: &Cases<T>, input: usize) -> bool {
    cases.cases.iter().any(|(c, _)| cond_needs_sorted(c, input))
}

/// `true` if a percentile of input `input` is used (its values are kept).
fn needs_sorted(rule: &CfRule, input: usize) -> bool {
    match &rule.action {
        Action::Background(ColorRule::Rules(c))
        | Action::FontColor(ColorRule::Rules(c))
        | Action::Fill(ColorRule::Rules(c)) => cases_need_sorted(c, input),
        Action::Icon(IconRule {
            source: IconSource::Rules(c),
            ..
        }) => cases_need_sorted(c, input),
        _ => false,
    }
}

impl Stats {
    /// Statistics of `columns` (bound inputs of `rule`) over the cells of
    /// `source` that are not totals.
    pub(super) fn collect(columns: &[usize], source: &dyn CellSource, rule: &CfRule) -> Self {
        let n = source.cell_count();
        let inputs = columns
            .iter()
            .enumerate()
            .map(|(k, &col)| {
                let sorted_wanted = needs_sorted(rule, k);
                let mut s = InputStats::default();
                for cell in (0..n).filter(|&c| !source.is_total(c)) {
                    let Some(x) = number(&source.value(col, cell)) else {
                        continue;
                    };
                    s.min = Some(s.min.map_or(x, |m| m.min(x)));
                    s.max = Some(s.max.map_or(x, |m| m.max(x)));
                    if sorted_wanted {
                        s.sorted.push(x);
                    }
                }
                s.sorted.sort_by(f64::total_cmp);
                s
            })
            .collect();
        Self { inputs }
    }

    fn input(&self, i: usize) -> Option<&InputStats> {
        self.inputs.get(i)
    }
}

/// Percentile of sorted values (`PERCENTILE.INC` / `PERCENTILE.EXC`).
fn percentile(sorted: &[f64], k: f64, exclusive: bool) -> Option<f64> {
    let n = sorted.len();
    if n == 0 || !k.is_finite() {
        return None;
    }
    let rank = if exclusive {
        if k <= 0.0 || k >= 1.0 {
            return None;
        }
        let r = k * (n as f64 + 1.0) - 1.0;
        if r < 0.0 || r > (n - 1) as f64 {
            return None;
        }
        r
    } else {
        k.clamp(0.0, 1.0) * (n - 1) as f64
    };
    let lo = rank.floor() as usize;
    let hi = (lo + 1).min(n - 1);
    let (a, b) = (*sorted.get(lo)?, *sorted.get(hi)?);
    Some(a + (b - a) * (rank - lo as f64))
}

/// An operand's value.
#[derive(Debug, Clone, PartialEq)]
enum Val {
    Null,
    Num(f64),
    Text(String),
    Bool(bool),
}

fn val_of(v: &Value) -> Option<Val> {
    Some(match v {
        Value::Blank => Val::Null,
        Value::Boolean(b) => Val::Bool(*b),
        Value::String(s) => Val::Text(s.to_string()),
        other => Val::Num(other.as_f64().filter(|x| x.is_finite())?),
    })
}

fn operand(op: &Operand, inputs: &[Value], stats: &Stats) -> Option<Val> {
    Some(match op {
        Operand::Input(i) => val_of(inputs.get(*i)?)?,
        Operand::Literal(l) => match l {
            LiteralValue::Null => Val::Null,
            LiteralValue::Text(s) => Val::Text(s.clone()),
            LiteralValue::Boolean(b) => Val::Bool(*b),
            LiteralValue::DateTime(_) => return None,
            other => Val::Num(other.as_f64()?),
        },
        Operand::Min(i) => Val::Num(stats.input(*i)?.min?),
        Operand::Max(i) => Val::Num(stats.input(*i)?.max?),
        Operand::Percentile {
            input,
            k,
            exclusive,
        } => Val::Num(percentile(&stats.input(*input)?.sorted, *k, *exclusive)?),
        Operand::Arith { op, left, right } => {
            let (Val::Num(a), Val::Num(b)) = (
                operand(left, inputs, stats)?,
                operand(right, inputs, stats)?,
            ) else {
                return None;
            };
            let r = match op {
                ArithOp::Add => a + b,
                ArithOp::Subtract => a - b,
                ArithOp::Multiply => a * b,
                ArithOp::Divide if b == 0.0 => return None,
                ArithOp::Divide => a / b,
            };
            Val::Num(r.is_finite().then_some(r)?)
        }
    })
}

fn compare(kind: ComparisonKind, a: &Val, b: &Val) -> bool {
    use std::cmp::Ordering;
    let ord = match (a, b) {
        (Val::Null, Val::Null) => return kind == ComparisonKind::Equal,
        (Val::Null, _) | (_, Val::Null) => return false,
        (Val::Num(x), Val::Num(y)) => x.partial_cmp(y),
        (Val::Text(x), Val::Text(y)) => Some(x.to_lowercase().cmp(&y.to_lowercase())),
        (Val::Bool(x), Val::Bool(y)) => Some(x.cmp(y)),
        _ => None,
    };
    let Some(ord) = ord else {
        return false;
    };
    match kind {
        ComparisonKind::Equal => ord == Ordering::Equal,
        ComparisonKind::GreaterThan => ord == Ordering::Greater,
        ComparisonKind::GreaterThanOrEqual => ord != Ordering::Less,
        ComparisonKind::LessThan => ord == Ordering::Less,
        ComparisonKind::LessThanOrEqual => ord != Ordering::Greater,
    }
}

fn holds(c: &Cond, inputs: &[Value], stats: &Stats) -> bool {
    match c {
        Cond::Compare { kind, left, right } => {
            match (operand(left, inputs, stats), operand(right, inputs, stats)) {
                (Some(a), Some(b)) => compare(*kind, &a, &b),
                _ => false,
            }
        }
        Cond::And(a, b) => holds(a, inputs, stats) && holds(b, inputs, stats),
        Cond::Or(a, b) => holds(a, inputs, stats) || holds(b, inputs, stats),
        Cond::Not(a) => !holds(a, inputs, stats),
    }
}

impl<T> Cases<T> {
    /// The value of the first case that holds, else the default.
    fn pick(&self, inputs: &[Value], stats: &Stats) -> Option<&T> {
        self.cases
            .iter()
            .find(|(c, _)| holds(c, inputs, stats))
            .map(|(_, v)| v)
            .or(self.default.as_ref())
    }
}

/// Position of `x` between `lo` and `hi` (0–1; ½ for an empty range).
fn unit(x: f64, lo: f64, hi: f64) -> f64 {
    if hi > lo {
        ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
    } else {
        0.5
    }
}

impl Gradient {
    fn paint(&self, v: &Value, stats: Option<&InputStats>, theme: &ThemeResolver) -> Option<Paint> {
        let x = match v {
            Value::Blank => match &self.nulls {
                NullColor::AsZero => 0.0,
                NullColor::NoColor => return None,
                NullColor::Color(c) => return theme.color(c).map(Paint::solid),
            },
            v => number(v)?,
        };
        let lo = self.min.value.or(stats.and_then(|s| s.min))?;
        let hi = self.max.value.or(stats.and_then(|s| s.max))?;
        let c_min = theme.color(&self.min.color)?;
        let c_max = theme.color(&self.max.color)?;
        let color = match &self.mid {
            None => lerp(c_min, c_max, unit(x, lo, hi)),
            Some(mid) => {
                let m = mid.value.unwrap_or((lo + hi) / 2.0);
                let c_mid = theme.color(&mid.color)?;
                if x <= m {
                    lerp(c_min, c_mid, unit(x, lo, m))
                } else {
                    lerp(c_mid, c_max, unit(x, m, hi))
                }
            }
        };
        Some(Paint::solid(color))
    }
}

impl ColorRule {
    /// The color of a cell with input values `inputs`.
    pub fn paint(&self, inputs: &[Value], stats: &Stats, theme: &ThemeResolver) -> Option<Paint> {
        match self {
            ColorRule::Gradient(g) => g.paint(inputs.first()?, stats.input(0), theme),
            ColorRule::Rules(cases) => cases
                .pick(inputs, stats)
                .and_then(|c| theme.color(c))
                .map(Paint::solid),
            ColorRule::FieldValue => match inputs.first()? {
                Value::String(s) => parse_css_color(s),
                _ => None,
            },
        }
    }
}

impl DataBars {
    /// The bar of a cell whose value is `v`.
    pub fn bar(&self, v: &Value, stats: &Stats, theme: &ThemeResolver) -> Option<Bar> {
        let x = number(v)?;
        let s = stats.input(0);
        let lo = self
            .min
            .unwrap_or_else(|| s.and_then(|s| s.min).unwrap_or(0.0).min(0.0));
        let hi = self
            .max
            .unwrap_or_else(|| s.and_then(|s| s.max).unwrap_or(0.0).max(0.0));
        if hi.partial_cmp(&lo) != Some(std::cmp::Ordering::Greater) {
            return None;
        }
        let frac = |v: f64| ((v - lo) / (hi - lo)).clamp(0.0, 1.0);
        let axis = frac(0.0);
        let end = frac(x);
        let (mut from, mut to) = if x >= 0.0 { (axis, end) } else { (end, axis) };
        let mut axis_at = axis;
        if self.reverse {
            (from, to) = (1.0 - to, 1.0 - from);
            axis_at = 1.0 - axis;
        }
        let color = |c: &Option<serde_json::Value>, default: Color| {
            c.as_ref().and_then(|c| theme.color(c)).unwrap_or(default)
        };
        let paint = if x >= 0.0 {
            Paint::solid(color(&self.positive, theme.data_color(0)))
        } else {
            Paint::solid(color(&self.negative, BAR_NEGATIVE))
        };
        Some(Bar {
            from,
            to,
            paint,
            axis: (lo < 0.0 && hi > 0.0).then(|| (axis_at, color(&self.axis, BAR_AXIS))),
        })
    }
}

impl IconRule {
    /// The icon of a cell with input values `inputs`.
    pub fn icon(&self, inputs: &[Value], stats: &Stats) -> Option<Icon> {
        match &self.source {
            IconSource::Rules(cases) => Icon::parse(cases.pick(inputs, stats)?),
            IconSource::FieldValue => Icon::parse(inputs.first()?.as_str()?),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles() {
        let v = [1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile(&v, 0.5, false), Some(2.5));
        assert_eq!(percentile(&v, 0.0, false), Some(1.0));
        assert_eq!(percentile(&v, 1.0, false), Some(4.0));
        assert_eq!(percentile(&v, 0.4, true), Some(2.0));
        assert_eq!(percentile(&v, 0.1, true), None);
        assert_eq!(percentile(&[], 0.5, false), None);
    }

    #[test]
    fn comparisons() {
        use ComparisonKind as K;
        assert!(compare(K::Equal, &Val::Null, &Val::Null));
        assert!(!compare(K::LessThan, &Val::Null, &Val::Num(1.0)));
        assert!(!compare(K::Equal, &Val::Num(0.0), &Val::Null));
        assert!(compare(
            K::Equal,
            &Val::Text("Ok".into()),
            &Val::Text("OK".into())
        ));
        assert!(compare(
            K::GreaterThanOrEqual,
            &Val::Num(2.0),
            &Val::Num(2.0)
        ));
        assert!(!compare(
            K::GreaterThan,
            &Val::Text("2".into()),
            &Val::Num(1.0)
        ));
    }
}
