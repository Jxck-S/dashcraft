//! Format-agnostic semantic-query expressions shared by the report AST, the
//! linter and the renderer.
//!
//! These are *not* raw PBIR serde types: `pbi-pbir` (and the legacy `Layout`
//! reader) resolve `SourceRef` aliases to table names and convert into these
//! types. Anything they cannot represent is kept as `Unknown(serde_json::Value)`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::ids::ObjectId;
use crate::macros::{lenient_eq, string_enum};
use crate::props::Origin;
use crate::serde_util::{is_false, is_null};

/// A field or value used by a visual, filter or sort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldExpr {
    /// Column reference; `entity` is the resolved table name.
    Column { entity: String, property: String },
    /// Measure reference; `entity` is the measure's home table.
    Measure { entity: String, property: String },
    /// Implicit aggregation of another expression (usually a column).
    Aggregation {
        expr: Box<FieldExpr>,
        function: AggregationFunction,
    },
    /// Level of a user hierarchy (or of an auto date/time variation).
    HierarchyLevel {
        entity: String,
        hierarchy: String,
        level: String,
        /// Set when the hierarchy hangs off a column variation (auto
        /// date/time): the PBIR `PropertyVariationSource`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        variation: Option<PropertyVariation>,
    },
    /// A constant.
    Literal(Literal),
    /// Any other semantic-query expression, verbatim.
    Unknown(serde_json::Value),
}

impl Default for FieldExpr {
    fn default() -> Self {
        FieldExpr::Unknown(serde_json::Value::Null)
    }
}

/// PBIR `PropertyVariationSource` (auto date/time hierarchies).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PropertyVariation {
    /// Variation name, e.g. `Variation`.
    pub name: String,
    /// Column the variation belongs to, e.g. `OrderDate`.
    pub property: String,
}

impl FieldExpr {
    pub fn column(entity: impl Into<String>, property: impl Into<String>) -> Self {
        FieldExpr::Column {
            entity: entity.into(),
            property: property.into(),
        }
    }

    pub fn measure(entity: impl Into<String>, property: impl Into<String>) -> Self {
        FieldExpr::Measure {
            entity: entity.into(),
            property: property.into(),
        }
    }

    pub fn aggregation(expr: FieldExpr, function: AggregationFunction) -> Self {
        FieldExpr::Aggregation {
            expr: Box::new(expr),
            function,
        }
    }

    /// The innermost expression below any aggregations.
    pub fn base(&self) -> &FieldExpr {
        let mut e = self;
        while let FieldExpr::Aggregation { expr, .. } = e {
            e = expr;
        }
        e
    }

    /// Table the expression refers to.
    pub fn entity(&self) -> Option<&str> {
        match self.base() {
            FieldExpr::Column { entity, .. }
            | FieldExpr::Measure { entity, .. }
            | FieldExpr::HierarchyLevel { entity, .. } => Some(entity),
            _ => None,
        }
    }

    /// Column/measure name (hierarchy level name for hierarchy levels).
    pub fn property(&self) -> Option<&str> {
        match self.base() {
            FieldExpr::Column { property, .. } | FieldExpr::Measure { property, .. } => {
                Some(property)
            }
            FieldExpr::HierarchyLevel { level, .. } => Some(level),
            _ => None,
        }
    }

    pub fn is_measure(&self) -> bool {
        matches!(self.base(), FieldExpr::Measure { .. })
    }

    /// The `queryRef` Power BI generates for this expression:
    /// `Stores.Region`, `CountNonNull(T.Status)`,
    /// `Calendar.Date Hierarchy.Year`. `None` for literals/unknown.
    pub fn query_ref(&self) -> Option<String> {
        match self {
            FieldExpr::Column { entity, property } | FieldExpr::Measure { entity, property } => {
                Some(format!("{entity}.{property}"))
            }
            FieldExpr::Aggregation { expr, function } => {
                let inner = expr.query_ref()?;
                Some(format!("{}({inner})", function.query_ref_name()?))
            }
            FieldExpr::HierarchyLevel {
                entity,
                hierarchy,
                level,
                variation,
            } => Some(match variation {
                Some(v) => format!("{entity}.{}.{hierarchy}.{level}", v.property),
                None => format!("{entity}.{hierarchy}.{level}"),
            }),
            FieldExpr::Literal(_) | FieldExpr::Unknown(_) => None,
        }
    }
}

/// Aggregation function of a PBIR `Aggregation` expression.
///
/// Codes follow the PBIR semantic-query schema (`QueryAggregateFunction`):
/// 0 Sum, 1 Avg, 2 DistinctCount, 3 Min, 4 Max, 5 CountNonNull (the UI's
/// "Count"), 6 Median, 7 StandardDeviation, 8 Variance.
/// Serialized as a camelCase name, or as the raw number for `Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AggregationFunction {
    Sum,
    Avg,
    DistinctCount,
    Min,
    Max,
    CountNonNull,
    Median,
    StdDev,
    Var,
    Unknown(i64),
}

impl AggregationFunction {
    const KNOWN: [AggregationFunction; 9] = [
        AggregationFunction::Sum,
        AggregationFunction::Avg,
        AggregationFunction::DistinctCount,
        AggregationFunction::Min,
        AggregationFunction::Max,
        AggregationFunction::CountNonNull,
        AggregationFunction::Median,
        AggregationFunction::StdDev,
        AggregationFunction::Var,
    ];

    /// From the PBIR numeric code.
    pub fn from_code(code: i64) -> Self {
        usize::try_from(code)
            .ok()
            .and_then(|i| Self::KNOWN.get(i).copied())
            .unwrap_or(AggregationFunction::Unknown(code))
    }

    /// PBIR numeric code.
    pub fn code(self) -> i64 {
        match self {
            AggregationFunction::Sum => 0,
            AggregationFunction::Avg => 1,
            AggregationFunction::DistinctCount => 2,
            AggregationFunction::Min => 3,
            AggregationFunction::Max => 4,
            AggregationFunction::CountNonNull => 5,
            AggregationFunction::Median => 6,
            AggregationFunction::StdDev => 7,
            AggregationFunction::Var => 8,
            AggregationFunction::Unknown(code) => code,
        }
    }

    /// Stable camelCase name (`sum`, `distinctCount`, …); `None` for `Unknown`.
    pub fn name(self) -> Option<&'static str> {
        Some(match self {
            AggregationFunction::Sum => "sum",
            AggregationFunction::Avg => "avg",
            AggregationFunction::DistinctCount => "distinctCount",
            AggregationFunction::Min => "min",
            AggregationFunction::Max => "max",
            AggregationFunction::CountNonNull => "countNonNull",
            AggregationFunction::Median => "median",
            AggregationFunction::StdDev => "stdDev",
            AggregationFunction::Var => "var",
            AggregationFunction::Unknown(_) => return None,
        })
    }

    /// Function name used inside Power BI `queryRef`s (`Sum(T.C)`,
    /// `Count(T.C)` for distinct count, `CountNonNull(T.C)`).
    pub fn query_ref_name(self) -> Option<&'static str> {
        Some(match self {
            AggregationFunction::Sum => "Sum",
            AggregationFunction::Avg => "Avg",
            AggregationFunction::DistinctCount => "Count",
            AggregationFunction::Min => "Min",
            AggregationFunction::Max => "Max",
            AggregationFunction::CountNonNull => "CountNonNull",
            AggregationFunction::Median => "Median",
            AggregationFunction::StdDev => "StandardDeviation",
            AggregationFunction::Var => "Variance",
            AggregationFunction::Unknown(_) => return None,
        })
    }

    /// Lenient name parse (`Sum`, `average`, `StandardDeviation`, …).
    pub fn parse_name(name: &str) -> Option<Self> {
        const ALIASES: [(&str, AggregationFunction); 6] = [
            ("average", AggregationFunction::Avg),
            ("standardDeviation", AggregationFunction::StdDev),
            ("variance", AggregationFunction::Var),
            ("count", AggregationFunction::CountNonNull),
            ("countDistinct", AggregationFunction::DistinctCount),
            ("distinct", AggregationFunction::DistinctCount),
        ];
        Self::KNOWN
            .iter()
            .filter_map(|f| f.name().map(|n| (n, *f)))
            .chain(ALIASES)
            .find(|(n, _)| lenient_eq(name, n))
            .map(|(_, f)| f)
    }
}

impl Serialize for AggregationFunction {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self.name() {
            Some(name) => serializer.serialize_str(name),
            None => serializer.serialize_i64(self.code()),
        }
    }
}

impl<'de> Deserialize<'de> for AggregationFunction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = AggregationFunction;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an aggregation function name or PBIR code")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                AggregationFunction::parse_name(v)
                    .ok_or_else(|| E::custom(format!("unknown aggregation function `{v}`")))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(AggregationFunction::from_code(v))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                i64::try_from(v)
                    .map(AggregationFunction::from_code)
                    .map_err(|_| E::custom("aggregation code out of range"))
            }
        }
        deserializer.deserialize_any(V)
    }
}

/// Error from [`Literal::value`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid semantic-query literal `{0}`")]
pub struct LiteralError(pub String);

/// A semantic-query literal, kept as the raw PBIR text (`'West'`,
/// `240L`, `1.5D`, `true`, `null`, `datetime'2026-01-01T00:00:00'`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Literal {
    pub raw: String,
}

impl Literal {
    pub fn new(raw: impl Into<String>) -> Self {
        Self { raw: raw.into() }
    }

    /// Parses the raw text into a typed value.
    pub fn value(&self) -> Result<LiteralValue, LiteralError> {
        LiteralValue::parse(&self.raw)
    }

    /// `'text'` with `'` doubled.
    pub fn text(value: &str) -> Self {
        Self::new(format!("'{}'", value.replace('\'', "''")))
    }

    pub fn integer(value: i64) -> Self {
        Self::new(format!("{value}L"))
    }

    pub fn double(value: f64) -> Self {
        Self::new(format!("{value}D"))
    }

    pub fn decimal(value: f64) -> Self {
        Self::new(format!("{value}M"))
    }

    pub fn boolean(value: bool) -> Self {
        Self::new(if value { "true" } else { "false" })
    }

    pub fn null() -> Self {
        Self::new("null")
    }

    /// `datetime'…'` from an ISO-8601 timestamp.
    pub fn datetime(iso: &str) -> Self {
        Self::new(format!("datetime'{iso}'"))
    }

    /// Encodes a typed value.
    pub fn from_value(value: &LiteralValue) -> Self {
        match value {
            LiteralValue::Text(s) => Self::text(s),
            LiteralValue::Integer(n) => Self::integer(*n),
            LiteralValue::Double(f) => Self::double(*f),
            LiteralValue::Decimal(f) => Self::decimal(*f),
            LiteralValue::Boolean(b) => Self::boolean(*b),
            LiteralValue::DateTime(s) => Self::datetime(s),
            LiteralValue::Null => Self::null(),
        }
    }
}

impl fmt::Display for Literal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

/// Typed value of a [`Literal`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LiteralValue {
    Text(String),
    Integer(i64),
    Double(f64),
    /// Fixed decimal (`M` suffix), as `f64`; the exact digits stay in
    /// [`Literal::raw`].
    Decimal(f64),
    Boolean(bool),
    /// ISO-8601 text between the quotes of `datetime'…'`.
    DateTime(String),
    Null,
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        s.get(prefix.len()..)
    } else {
        None
    }
}

/// Inner text of `'…'` with `''` unescaped; `None` if not a well-formed
/// single-quoted string.
fn unquote_single(s: &str) -> Option<String> {
    let inner = s.strip_prefix('\'')?.strip_suffix('\'')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\'' {
            // Inside the literal a quote must be doubled.
            if chars.next() != Some('\'') {
                return None;
            }
        }
        out.push(c);
    }
    Some(out)
}

impl LiteralValue {
    /// Parses PBIR literal text (see [`Literal`]).
    pub fn parse(raw: &str) -> Result<Self, LiteralError> {
        let err = || LiteralError(raw.to_owned());
        let s = raw.trim();
        if s.starts_with('\'') {
            return unquote_single(s).map(LiteralValue::Text).ok_or_else(err);
        }
        for prefix in ["datetimeoffset", "datetime"] {
            if let Some(rest) = strip_prefix_ci(s, prefix)
                && rest.starts_with('\'')
            {
                return unquote_single(rest)
                    .map(LiteralValue::DateTime)
                    .ok_or_else(err);
            }
        }
        if s.eq_ignore_ascii_case("true") {
            return Ok(LiteralValue::Boolean(true));
        }
        if s.eq_ignore_ascii_case("false") {
            return Ok(LiteralValue::Boolean(false));
        }
        if s.eq_ignore_ascii_case("null") {
            return Ok(LiteralValue::Null);
        }
        let (number, suffix) = match s.char_indices().last() {
            Some((i, c)) if c.is_ascii_alphabetic() => (&s[..i], Some(c.to_ascii_uppercase())),
            _ => (s, None),
        };
        let looks_numeric = number.bytes().any(|b| b.is_ascii_digit())
            && number
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'));
        if !looks_numeric {
            return Err(err());
        }
        match suffix {
            Some('L') => number.parse().map(LiteralValue::Integer).map_err(|_| err()),
            Some('D') => number.parse().map(LiteralValue::Double).map_err(|_| err()),
            Some('M') => number.parse().map(LiteralValue::Decimal).map_err(|_| err()),
            Some(_) => Err(err()),
            None => number
                .parse()
                .map(LiteralValue::Integer)
                .or_else(|_| number.parse().map(LiteralValue::Double))
                .map_err(|_| err()),
        }
    }

    /// Text payload of `Text`.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            LiteralValue::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Numeric payload of `Integer`, `Double` or `Decimal`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            // Precision loss above 2^53 is acceptable for display/filtering.
            #[allow(clippy::cast_precision_loss)]
            LiteralValue::Integer(n) => Some(*n as f64),
            LiteralValue::Double(f) | LiteralValue::Decimal(f) => Some(*f),
            _ => None,
        }
    }
}

/// Comparison operator (PBIR `ComparisonKind` codes 0–4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComparisonKind {
    Equal,
    GreaterThan,
    GreaterThanOrEqual,
    LessThan,
    LessThanOrEqual,
}

impl ComparisonKind {
    /// From the PBIR numeric code.
    pub fn from_code(code: i64) -> Option<Self> {
        Some(match code {
            0 => ComparisonKind::Equal,
            1 => ComparisonKind::GreaterThan,
            2 => ComparisonKind::GreaterThanOrEqual,
            3 => ComparisonKind::LessThan,
            4 => ComparisonKind::LessThanOrEqual,
            _ => return None,
        })
    }

    pub fn code(self) -> i64 {
        match self {
            ComparisonKind::Equal => 0,
            ComparisonKind::GreaterThan => 1,
            ComparisonKind::GreaterThanOrEqual => 2,
            ComparisonKind::LessThan => 3,
            ComparisonKind::LessThanOrEqual => 4,
        }
    }
}

/// A boolean filter condition.
///
/// `right` operands are [`FieldExpr`]s rather than bare literals because
/// PBIR allows any expression there (`DateSpan`, `Now`, …); the common case
/// is `FieldExpr::Literal`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Condition {
    /// Tuple membership: `(exprs…) IN {(values…), …}`.
    In {
        exprs: Vec<FieldExpr>,
        values: Vec<Vec<Literal>>,
    },
    Comparison {
        kind: ComparisonKind,
        left: FieldExpr,
        right: FieldExpr,
    },
    And(Box<Condition>, Box<Condition>),
    Or(Box<Condition>, Box<Condition>),
    Not(Box<Condition>),
    Contains {
        left: FieldExpr,
        right: FieldExpr,
    },
    StartsWith {
        left: FieldExpr,
        right: FieldExpr,
    },
    Between {
        expr: FieldExpr,
        lower: FieldExpr,
        upper: FieldExpr,
    },
    /// Anything else (`Exists`, `In` over a table, …), verbatim.
    Unknown(serde_json::Value),
}

impl Condition {
    /// Combines conditions with `And` (left-nested). `None` if empty.
    pub fn and_all(conditions: impl IntoIterator<Item = Condition>) -> Option<Condition> {
        conditions
            .into_iter()
            .reduce(|a, b| Condition::And(Box::new(a), Box::new(b)))
    }

    /// Visits every field expression in the condition (not descending into
    /// aggregations; use [`FieldExpr::base`]).
    pub fn visit_fields(&self, f: &mut dyn FnMut(&FieldExpr)) {
        match self {
            Condition::In { exprs, .. } => {
                for e in exprs {
                    f(e);
                }
            }
            Condition::Comparison { left, right, .. }
            | Condition::Contains { left, right }
            | Condition::StartsWith { left, right } => {
                f(left);
                f(right);
            }
            Condition::Between { expr, lower, upper } => {
                f(expr);
                f(lower);
                f(upper);
            }
            Condition::And(a, b) | Condition::Or(a, b) => {
                a.visit_fields(f);
                b.visit_fields(f);
            }
            Condition::Not(c) => c.visit_fields(f),
            Condition::Unknown(_) => {}
        }
    }

    /// Mutable variant of [`Condition::visit_fields`].
    pub fn visit_fields_mut(&mut self, f: &mut dyn FnMut(&mut FieldExpr)) {
        match self {
            Condition::In { exprs, .. } => {
                for e in exprs {
                    f(e);
                }
            }
            Condition::Comparison { left, right, .. }
            | Condition::Contains { left, right }
            | Condition::StartsWith { left, right } => {
                f(left);
                f(right);
            }
            Condition::Between { expr, lower, upper } => {
                f(expr);
                f(lower);
                f(upper);
            }
            Condition::And(a, b) | Condition::Or(a, b) => {
                a.visit_fields_mut(f);
                b.visit_fields_mut(f);
            }
            Condition::Not(c) => c.visit_fields_mut(f),
            Condition::Unknown(_) => {}
        }
    }
}

string_enum! {
    /// PBIR filter `type`.
    #[derive(Default)]
    pub enum FilterKind {
        #[default]
        Categorical = "Categorical",
        Advanced = "Advanced",
        TopN = "TopN",
        RelativeDate = "RelativeDate",
        RelativeTime = "RelativeTime",
        Include = "Include",
        Exclude = "Exclude",
        Passthrough = "Passthrough",
        Tuple = "Tuple",
    }
}

/// A report-, page- or visual-level filter.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filter {
    pub id: ObjectId,
    /// PBIR filter `name` (unique within its container).
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// The field shown in the filter pane.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<FieldExpr>,
    pub kind: FilterKind,
    /// The active condition; several PBIR `Where` clauses are combined with
    /// `And`. `None` means the filter card is present but not filtering.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<Condition>,
    #[serde(skip_serializing_if = "is_false")]
    pub is_hidden_in_view_mode: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub is_locked_in_view_mode: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub how_created: Option<String>,
    /// The source filter JSON, verbatim, for lossless writing. Writers
    /// regenerate `field`/`condition` from the typed fields and take
    /// everything else from here.
    #[serde(skip_serializing_if = "is_null")]
    pub raw: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl Filter {
    /// Visits `field` and every field in `condition`.
    pub fn visit_fields(&self, f: &mut dyn FnMut(&FieldExpr)) {
        if let Some(field) = &self.field {
            f(field);
        }
        if let Some(c) = &self.condition {
            c.visit_fields(f);
        }
    }

    /// Mutable variant of [`Filter::visit_fields`].
    pub fn visit_fields_mut(&mut self, f: &mut dyn FnMut(&mut FieldExpr)) {
        if let Some(field) = &mut self.field {
            f(field);
        }
        if let Some(c) = &mut self.condition {
            c.visit_fields_mut(f);
        }
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

/// One entry of a visual's sort definition.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SortItem {
    pub field: FieldExpr,
    pub direction: SortDirection,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(raw: &str) -> LiteralValue {
        Literal::new(raw).value().unwrap()
    }

    #[test]
    fn literals_parse_into_typed_values() {
        assert_eq!(lit("'West'"), LiteralValue::Text("West".into()));
        assert_eq!(lit("'O''Brien'"), LiteralValue::Text("O'Brien".into()));
        assert_eq!(lit("''"), LiteralValue::Text(String::new()));
        assert_eq!(lit("240L"), LiteralValue::Integer(240));
        assert_eq!(lit("-5L"), LiteralValue::Integer(-5));
        assert_eq!(lit("1.5D"), LiteralValue::Double(1.5));
        assert_eq!(lit("1.5M"), LiteralValue::Decimal(1.5));
        assert_eq!(lit("1.5"), LiteralValue::Double(1.5));
        assert_eq!(lit("12"), LiteralValue::Integer(12));
        assert_eq!(lit("1E+3D"), LiteralValue::Double(1000.0));
        assert_eq!(lit("true"), LiteralValue::Boolean(true));
        assert_eq!(lit("FALSE"), LiteralValue::Boolean(false));
        assert_eq!(lit("null"), LiteralValue::Null);
        assert_eq!(
            lit("datetime'2026-01-01T00:00:00'"),
            LiteralValue::DateTime("2026-01-01T00:00:00".into())
        );
    }

    #[test]
    fn malformed_literals_are_errors_not_panics() {
        for raw in [
            "",
            "'unterminated",
            "'a'b'",
            "abc",
            "1.5X",
            "L",
            "infD",
            "NaND",
            "datetime'x",
            "€",
            "1.2.3D",
            "9999999999999999999999L",
        ] {
            assert!(Literal::new(raw).value().is_err(), "raw {raw:?}");
        }
    }

    #[test]
    fn literal_constructors_round_trip() {
        let values = [
            LiteralValue::Text("It's".into()),
            LiteralValue::Integer(240),
            LiteralValue::Double(0.25),
            LiteralValue::Decimal(12.5),
            LiteralValue::Boolean(true),
            LiteralValue::DateTime("2026-10-21T00:00:00".into()),
            LiteralValue::Null,
        ];
        for v in values {
            assert_eq!(Literal::from_value(&v).value().unwrap(), v);
        }
        assert_eq!(Literal::text("It's").raw, "'It''s'");
        assert_eq!(Literal::integer(240).raw, "240L");
    }

    #[test]
    fn aggregation_codes_names_and_query_refs() {
        for code in 0..9 {
            assert_eq!(AggregationFunction::from_code(code).code(), code);
        }
        assert_eq!(
            AggregationFunction::from_code(5),
            AggregationFunction::CountNonNull
        );
        assert_eq!(
            AggregationFunction::from_code(42),
            AggregationFunction::Unknown(42)
        );
        assert_eq!(AggregationFunction::from_code(-1).code(), -1);

        let field = FieldExpr::aggregation(
            FieldExpr::column("Web Sessions", "Device"),
            AggregationFunction::CountNonNull,
        );
        assert_eq!(
            field.query_ref().as_deref(),
            Some("CountNonNull(Web Sessions.Device)")
        );
        assert_eq!(field.entity(), Some("Web Sessions"));
        assert_eq!(field.property(), Some("Device"));
        assert!(!field.is_measure());

        let json = serde_json::to_string(&[
            AggregationFunction::DistinctCount,
            AggregationFunction::Unknown(12),
        ])
        .unwrap();
        assert_eq!(json, r#"["distinctCount",12]"#);
        let back: Vec<AggregationFunction> =
            serde_json::from_str(r#"["distinctCount",12,"Average",0]"#).unwrap();
        assert_eq!(
            back,
            [
                AggregationFunction::DistinctCount,
                AggregationFunction::Unknown(12),
                AggregationFunction::Avg,
                AggregationFunction::Sum
            ]
        );
        assert!(serde_json::from_str::<AggregationFunction>(r#""bogus""#).is_err());
    }

    #[test]
    fn hierarchy_level_query_refs() {
        let level = FieldExpr::HierarchyLevel {
            entity: "Sales".into(),
            hierarchy: "Date Hierarchy".into(),
            level: "Year".into(),
            variation: Some(PropertyVariation {
                name: "Variation".into(),
                property: "OrderDate".into(),
            }),
        };
        assert_eq!(
            level.query_ref().as_deref(),
            Some("Sales.OrderDate.Date Hierarchy.Year")
        );
        assert_eq!(level.property(), Some("Year"));
        assert_eq!(FieldExpr::Literal(Literal::null()).query_ref(), None);
    }

    #[test]
    fn condition_visitors_reach_every_field() {
        let cond = Condition::and_all([
            Condition::In {
                exprs: vec![FieldExpr::column("Orders", "Status")],
                values: vec![vec![Literal::text("Cancelled")]],
            },
            Condition::Not(Box::new(Condition::Comparison {
                kind: ComparisonKind::Equal,
                left: FieldExpr::column("Orders", "Discount"),
                right: FieldExpr::Literal(Literal::null()),
            })),
            Condition::Between {
                expr: FieldExpr::column("Orders", "Quantity"),
                lower: FieldExpr::Literal(Literal::integer(1)),
                upper: FieldExpr::Literal(Literal::integer(5)),
            },
        ])
        .unwrap();
        let mut props = Vec::new();
        cond.visit_fields(&mut |f| props.extend(f.property().map(str::to_owned)));
        assert_eq!(props, ["Status", "Discount", "Quantity"]);

        let mut cond = cond;
        cond.visit_fields_mut(&mut |f| {
            if let FieldExpr::Column { entity, .. } = f {
                *entity = "Sales".into();
            }
        });
        let mut entities = Vec::new();
        cond.visit_fields(&mut |f| entities.extend(f.entity().map(str::to_owned)));
        assert_eq!(entities, ["Sales", "Sales", "Sales"]);
        assert!(Condition::and_all([]).is_none());
        assert_eq!(ComparisonKind::from_code(3), Some(ComparisonKind::LessThan));
        assert_eq!(ComparisonKind::from_code(9), None);
    }
}
