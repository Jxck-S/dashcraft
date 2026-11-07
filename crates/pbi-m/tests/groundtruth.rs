//! Differential tests of the M engine against Power BI Desktop's mashup engine (ADR-0011).
//!
//! `reference/groundtruth/corpus/<suite>.json` (`"language": "m"`) lists M expressions;
//! `reference/groundtruth/recordings/<suite>.json` holds the value (or error) Desktop's mashup
//! engine gave each one, encoded as typed JSON by the probe of
//! `scripts/groundtruth/GroundTruth.psm1`. Each test evaluates the suite's expressions with
//! [`pbi_m::Engine`], encodes the results the same way and compares them:
//!
//! * values structurally: record fields in order, numbers as doubles within 1e-12 relative
//!   (Desktop computes some results as decimals), text, dates and times exactly;
//! * errors by `Reason: Message`; a Desktop syntax error (`[line,col-line,col] …`) only needs a
//!   DashCraft syntax error;
//! * a slot that holds an error makes the whole case that error, as in Desktop's probe.
//!
//! A mismatch fails unless `reference/groundtruth/deviations.json` lists it with a reason; a
//! listed case that now matches fails too (the ledger only shrinks). Reports go to
//! `target/tmp/groundtruth/<suite>.md`; a missing recording prints `SKIP: <file> missing`.
//!
//! The `m-functions` cases also run after [`pbi_m::service::format`] lays them out (at the
//! default width and a narrow one with tabs): the formatted queries must give Desktop's
//! recorded results with the same ledger.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use chrono::{NaiveTime, TimeDelta, Timelike};
use pbi_m::{Engine, Lazy, MError, TypeValue, Value};
use serde_json::{Map, Value as Json};

fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn report_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/groundtruth")
}

fn read_json(path: &Path) -> Json {
    serde_json::from_slice(&std::fs::read(path).unwrap())
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// What DashCraft gave for a case.
#[derive(Debug, PartialEq)]
enum Outcome {
    Value(Json),
    /// `Reason: Message`.
    Error(String),
    Syntax(String),
}

fn error_text(e: &MError) -> String {
    match e {
        MError::Evaluation {
            reason, message, ..
        } => format!("{reason}: {message}"),
        other => other.to_string(),
    }
}

/// `HH:mm:ss.FFFFFFF`: 100-ns ticks with trailing zeros trimmed, no dot when zero.
fn fraction(nanos: u32) -> String {
    let ticks = nanos / 100;
    if ticks == 0 {
        return String::new();
    }
    let digits = format!("{ticks:07}");
    format!(".{}", digits.trim_end_matches('0'))
}

fn time_text(t: NaiveTime) -> String {
    format!(
        "{}{}",
        t.format("%H:%M:%S"),
        fraction(t.nanosecond() % 1_000_000_000)
    )
}

/// .NET's constant `TimeSpan` format, as `Duration.ToText`: `[-][d.]hh:mm:ss[.fffffff]`.
fn duration_text(d: TimeDelta) -> String {
    let negative = d < TimeDelta::zero();
    let d = d.abs();
    let secs = d.num_seconds();
    let ticks = d.subsec_nanos() / 100;
    let mut out = String::new();
    if negative {
        out.push('-');
    }
    if secs >= 86_400 {
        let _ = write!(out, "{}.", secs / 86_400);
    }
    let _ = write!(
        out,
        "{:02}:{:02}:{:02}",
        secs % 86_400 / 3600,
        secs % 3600 / 60,
        secs % 60
    );
    if ticks > 0 {
        let _ = write!(out, ".{ticks:07}");
    }
    out
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18u32, 12, 6, 0].into_iter().enumerate() {
            out.push(if i <= chunk.len() {
                char::from(ALPHABET[((n >> shift) & 63) as usize])
            } else {
                '='
            });
        }
    }
    out
}

fn tag(kind: &str, body: Json) -> Json {
    let mut m = Map::new();
    m.insert(format!("#{kind}"), body);
    Json::Object(m)
}

fn type_name(t: &TypeValue) -> String {
    let base = t.base().name();
    match base {
        "any" | "null" | "none" => base.to_owned(),
        _ if t.is_nullable() => format!("nullable {base}"),
        _ => base.to_owned(),
    }
}

fn slot(l: &Lazy) -> Result<Json, MError> {
    encode(&l.resolved()?)
}

/// A value as the Desktop probe's `Encode` writes it.
fn encode(v: &Value) -> Result<Json, MError> {
    Ok(match v.plain() {
        Value::Null => Json::Null,
        Value::Logical(b) => Json::Bool(*b),
        Value::Number(n) if n.is_nan() => tag("number", Json::from("NaN")),
        Value::Number(n) if n.is_infinite() => tag(
            "number",
            Json::from(if *n > 0.0 { "Infinity" } else { "-Infinity" }),
        ),
        Value::Number(n) if n.fract() == 0.0 && n.abs() < 9e15 => Json::from(*n as i64),
        Value::Number(n) => Json::from(*n),
        Value::Text(t) => Json::from(t.as_ref()),
        Value::Date(d) => tag("date", Json::from(d.format("%Y-%m-%d").to_string())),
        Value::DateTime(dt) => tag(
            "datetime",
            Json::from(format!(
                "{}T{}",
                dt.format("%Y-%m-%d"),
                time_text(dt.time())
            )),
        ),
        Value::DateTimeZone(dt) => tag(
            "datetimezone",
            Json::from(format!(
                "{}T{}{}",
                dt.format("%Y-%m-%d"),
                time_text(dt.time()),
                dt.format("%:z")
            )),
        ),
        Value::Time(t) => tag("time", Json::from(time_text(*t))),
        Value::Duration(d) => tag("duration", Json::from(duration_text(*d))),
        Value::Binary(b) => tag("binary", Json::from(base64(&b.bytes(u64::MAX)?))),
        Value::List(l) => Json::Array(l.items().iter().map(slot).collect::<Result<_, _>>()?),
        Value::Table(t) => {
            let columns = t
                .column_names()
                .iter()
                .map(|c| Json::from(c.as_ref()))
                .collect();
            let mut rows = Vec::with_capacity(t.row_count());
            for r in 0..t.row_count() {
                let row = (0..t.column_count())
                    .map(|c| slot(t.cell(r, c).expect("cell in range")))
                    .collect::<Result<_, _>>()?;
                rows.push(Json::Array(row));
            }
            let mut body = Map::new();
            body.insert("columns".into(), Json::Array(columns));
            body.insert("rows".into(), Json::Array(rows));
            tag("table", Json::Object(body))
        }
        Value::Record(r) => {
            let mut body = Map::new();
            for (name, value) in r.iter() {
                body.insert(name.to_string(), slot(value)?);
            }
            if r.len() == 1 && r.names()[0].starts_with('#') {
                tag("record", Json::Object(body))
            } else {
                Json::Object(body)
            }
        }
        Value::Type(t) => tag("type", Json::from(type_name(t))),
        Value::Function(_) => tag("function", Json::Null),
        Value::Meta(_) => unreachable!("plain() looks through metadata"),
    })
}

/// Evaluates a case as the recorder does (`scripts/groundtruth/GroundTruth.psm1`):
/// `Expression.Evaluate(case, Library)` with the dotted names of `#shared` as the environment, the
/// case written into an M text literal with its quotes doubled. A case that does not parse is a
/// syntax error (Desktop raises `[l,c-l,c] Token ... expected.`; the messages are not compared).
fn evaluate(engine: &Engine, query: &str) -> Outcome {
    if let Err(e) = pbi_m::parse_expression(query) {
        return Outcome::Syntax(e.to_string());
    }
    let probe = format!(
        "let Library = Record.SelectFields(#shared, List.Select(Record.FieldNames(#shared), \
         each Text.Contains(_, \".\"))) in Expression.Evaluate(\"{}\", Library)",
        query.replace('"', "\"\"")
    );
    let result = engine.evaluate(&probe).and_then(|v| encode(&v));
    match result {
        Ok(json) => Outcome::Value(json),
        Err(e @ MError::Syntax { .. }) => Outcome::Syntax(e.to_string()),
        Err(e) => Outcome::Error(error_text(&e)),
    }
}

fn numbers_match(a: f64, b: f64) -> bool {
    a == b || (a - b).abs() <= 1e-12 * a.abs().max(b.abs())
}

fn same(recorded: &Json, ours: &Json) -> bool {
    match (recorded, ours) {
        (Json::Number(a), Json::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => numbers_match(a, b),
            _ => false,
        },
        (Json::Array(a), Json::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same(x, y))
        }
        (Json::Object(a), Json::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|((ka, va), (kb, vb))| ka == kb && same(va, vb))
        }
        (a, b) => a == b,
    }
}

/// Desktop reports syntax errors as `Expression.Error: [1,27-1,32] Token Literal expected.`.
fn is_desktop_syntax_error(e: &str) -> bool {
    e.strip_prefix("Expression.Error: [")
        .and_then(|rest| rest.split_once("] "))
        .is_some_and(|(span, _)| {
            !span.is_empty()
                && span
                    .chars()
                    .all(|c| c.is_ascii_digit() || c == ',' || c == '-')
        })
}

fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_owned();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// `Expression.Error: [2,9-2,19] The name …` without its `[2,9-2,19] ` source position.
fn without_position(e: &str) -> String {
    match e.split_once(": [").and_then(|(reason, rest)| {
        let (span, message) = rest.split_once("] ")?;
        let position = !span.is_empty()
            && span
                .chars()
                .all(|c| c.is_ascii_digit() || c == ',' || c == '-');
        position.then(|| format!("{reason}: {message}"))
    }) {
        Some(stripped) => stripped,
        None => e.to_owned(),
    }
}

/// `None` when DashCraft agrees with the recording, else what differs. With `moved`, the case
/// was rewritten, so the source positions in errors are not compared.
fn compare(recorded: &Json, ours: &Outcome, moved: bool) -> Option<String> {
    if let Some(err) = recorded.get("error").and_then(Json::as_str) {
        return match ours {
            Outcome::Error(e) if e == err => None,
            Outcome::Error(e) if moved && without_position(e) == without_position(err) => None,
            Outcome::Syntax(_) if is_desktop_syntax_error(err) => None,
            Outcome::Error(e) | Outcome::Syntax(e) => {
                Some(format!("error {e:?}, Desktop error {err:?}"))
            }
            Outcome::Value(v) => Some(format!(
                "value {}, Desktop error {err:?}",
                clip(&v.to_string(), 300)
            )),
        };
    }
    let expected = recorded.get("value");
    match (ours, expected) {
        (Outcome::Error(e) | Outcome::Syntax(e), _) => match expected {
            Some(v) => Some(format!(
                "error {e:?}, Desktop value {}",
                clip(&v.to_string(), 300)
            )),
            None => Some(format!("error {e:?}, Desktop succeeded")),
        },
        // Volatile: Desktop's value was not recorded, only that it succeeded.
        (Outcome::Value(_), None) => None,
        (Outcome::Value(v), Some(want)) if same(want, v) => None,
        (Outcome::Value(v), Some(want)) => Some(format!(
            "value {}, Desktop {}",
            clip(&v.to_string(), 300),
            clip(&want.to_string(), 300)
        )),
    }
}

#[derive(Debug, Clone)]
struct Deviation {
    case: String,
    kind: String,
}

impl Deviation {
    fn covers(&self, id: &str) -> bool {
        match self.case.strip_suffix('*') {
            Some(prefix) => id.starts_with(prefix),
            None => self.case == id,
        }
    }
}

const DEVIATION_KINDS: [&str; 4] = ["bug", "unsupported", "byDesign", "lenient"];

fn ledger(suite: &str) -> Vec<Deviation> {
    let path = reference_root().join("groundtruth/deviations.json");
    if !path.exists() {
        return Vec::new();
    }
    read_json(&path)["deviations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["suite"] == suite)
        .map(|d| {
            let dev = Deviation {
                case: d["case"].as_str().unwrap_or_default().to_owned(),
                kind: d["kind"].as_str().unwrap_or_default().to_owned(),
            };
            assert!(
                DEVIATION_KINDS.contains(&dev.kind.as_str()),
                "deviations.json: {suite}/{}: kind must be one of {DEVIATION_KINDS:?}",
                dev.case
            );
            assert!(
                d["reason"].as_str().is_some_and(|r| !r.trim().is_empty()),
                "deviations.json: {suite}/{}: a reason is required",
                dev.case
            );
            dev
        })
        .collect()
}

/// Rewrites a case before it is evaluated; `None` evaluates it as recorded.
type Transform = fn(&str) -> String;

fn run_suite(suite: &str) {
    run_suite_as(suite, suite, None);
}

/// Runs `suite` with each case rewritten by `transform`, reported as `report`.
fn run_suite_as(suite: &str, report: &str, transform: Option<Transform>) {
    let root = reference_root().join("groundtruth");
    let corpus_path = root.join(format!("corpus/{suite}.json"));
    let recording_path = root.join(format!("recordings/{suite}.json"));
    for p in [&corpus_path, &recording_path] {
        if !p.exists() {
            println!("SKIP: {} missing", p.display());
            return;
        }
    }
    let corpus = read_json(&corpus_path);
    assert_eq!(corpus["language"], "m", "{suite} is not an M suite");
    let recording = read_json(&recording_path);
    let recorded: BTreeMap<&str, &Json> = recording["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), c))
        .collect();
    let deviations = ledger(suite);
    let engine = Engine::default();

    let mut not_recorded = Vec::new();
    let mut stale_query = Vec::new();
    let mut passed = 0usize;
    let mut mismatches: Vec<(String, String)> = Vec::new();
    let cases = corpus["cases"].as_array().unwrap();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let Some(rec) = recorded.get(id) else {
            not_recorded.push(id.to_owned());
            continue;
        };
        if rec["query"] != case["query"] {
            stale_query.push(id.to_owned());
            continue;
        }
        let query = case["query"].as_str().unwrap();
        let ours = match transform {
            Some(f) => evaluate(&engine, &f(query)),
            None => evaluate(&engine, query),
        };
        match compare(rec, &ours, transform.is_some()) {
            None => passed += 1,
            Some(detail) => mismatches.push((id.to_owned(), detail)),
        }
    }

    let new: Vec<_> = mismatches
        .iter()
        .filter(|(id, _)| !deviations.iter().any(|d| d.covers(id)))
        .collect();
    let stale: Vec<_> = deviations
        .iter()
        .filter(|d| !mismatches.iter().any(|(id, _)| d.covers(id)))
        .collect();

    let mut md = String::new();
    let _ = writeln!(md, "# Ground truth: {report}\n");
    let _ = writeln!(
        md,
        "Desktop {} (engine {}), recorded {}.\n",
        recording["desktop"]["version"].as_str().unwrap_or("?"),
        recording["desktop"]["engine"].as_str().unwrap_or("?"),
        recording["recordedOn"].as_str().unwrap_or("?")
    );
    let _ = writeln!(
        md,
        "| cases | match | listed deviations | new mismatches | stale ledger entries | not recorded | query changed |\n|---|---|---|---|---|---|---|\n| {} | {passed} | {} | {} | {} | {} | {} |\n",
        cases.len(),
        mismatches.len() - new.len(),
        new.len(),
        stale.len(),
        not_recorded.len(),
        stale_query.len()
    );
    if !mismatches.is_empty() {
        let _ = writeln!(md, "| case | ledger | detail |\n|---|---|---|");
        for (id, detail) in &mismatches {
            let listed = deviations
                .iter()
                .find(|d| d.covers(id))
                .map_or("**new**".to_owned(), |d| d.kind.clone());
            let _ = writeln!(md, "| `{id}` | {listed} | {} |", detail.replace('|', "\\|"));
        }
    }
    let dir = report_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("{report}.md")), &md);
    println!(
        "{report}: {} cases, {passed} match, {} listed, {} new, {} stale; report {}",
        cases.len(),
        mismatches.len() - new.len(),
        new.len(),
        stale.len(),
        dir.join(format!("{report}.md")).display()
    );

    let mut failures = Vec::new();
    for (id, detail) in &new {
        failures.push(format!("  NEW {id}: {}", clip(detail, 400)));
    }
    for d in &stale {
        failures.push(format!(
            "  STALE ledger entry {} ({}): it matches Desktop now; remove it",
            d.case, d.kind
        ));
    }
    for id in &not_recorded {
        failures.push(format!(
            "  NOT RECORDED {id}: record the suite again (scripts/groundtruth/Record-GroundTruth.ps1)"
        ));
    }
    for id in &stale_query {
        failures.push(format!(
            "  QUERY CHANGED {id}: the corpus query differs from the recorded one; record again"
        ));
    }
    assert!(
        failures.is_empty(),
        "ground truth {report}: {} problem(s) (fix them, or list them in reference/groundtruth/deviations.json with a reason):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn m_functions() {
    run_suite("m-functions");
}

/// A case laid out by the formatter; a case that does not parse stays as it is (Desktop's
/// recording is its syntax error).
fn formatted(query: &str, opts: &pbi_m::service::FormatOptions) -> String {
    match pbi_m::service::format(query, opts) {
        Ok(out) => {
            assert_eq!(
                pbi_m::service::format(&out, opts).as_deref(),
                Ok(out.as_str()),
                "not idempotent: {query}"
            );
            out
        }
        Err(pbi_m::service::FormatError::Syntax(_)) => query.to_owned(),
        Err(e) => panic!("{e}: {query}"),
    }
}

#[test]
fn m_functions_formatted() {
    run_suite_as(
        "m-functions",
        "m-functions-formatted",
        Some(|q| formatted(q, &pbi_m::service::FormatOptions::default())),
    );
    run_suite_as(
        "m-functions",
        "m-functions-formatted-narrow",
        Some(|q| {
            formatted(
                q,
                &pbi_m::service::FormatOptions {
                    max_width: 20,
                    indent_width: 2,
                    use_tabs: true,
                },
            )
        }),
    );
}

#[test]
fn encoding_matches_the_desktop_probe() {
    let engine = Engine::default();
    let value = |q: &str| match evaluate(&engine, q) {
        Outcome::Value(v) => v.to_string(),
        other => panic!("{q}: {other:?}"),
    };
    assert_eq!(
        value("#datetime(2024, 1, 2, 3, 4, 5.25)"),
        r##"{"#datetime":"2024-01-02T03:04:05.25"}"##
    );
    assert_eq!(
        value("#datetimezone(2024, 1, 2, 3, 4, 5, -1, -30)"),
        r##"{"#datetimezone":"2024-01-02T03:04:05-01:30"}"##
    );
    assert_eq!(
        value("{#duration(-1, -2, 0, 0), #duration(0, 0, 0, 1.5)}"),
        r##"[{"#duration":"-1.02:00:00"},{"#duration":"00:00:01.5000000"}]"##
    );
    assert_eq!(value("#binary({1, 2})"), r##"{"#binary":"AQI="}"##);
    assert_eq!(value(r##"[#"#a" = 1]"##), r##"{"#record":{"#a":1}}"##);
    assert_eq!(
        value("{type nullable text, type any, each _}"),
        r##"[{"#type":"nullable text"},{"#type":"any"},{"#function":null}]"##
    );
    // An error in a slot is the case's error.
    assert_eq!(
        evaluate(&engine, r#"{1, error "slot"}"#),
        Outcome::Error("Expression.Error: slot".into())
    );
    assert!(matches!(evaluate(&engine, "1 +"), Outcome::Syntax(_)));
}

#[test]
fn comparison_rules() {
    let rec = |j: &str| serde_json::from_str::<Json>(j).unwrap();
    let ok = |j: &str| Outcome::Value(serde_json::from_str(j).unwrap());
    // Decimal results within 1e-12 relative.
    assert_eq!(
        compare(
            &rec(r#"{"value": 2.333333333333333}"#),
            &ok("2.3333333333333335"),
            false
        ),
        None
    );
    assert!(compare(&rec(r#"{"value": 2.3333}"#), &ok("2.3334"), false).is_some());
    // Record fields compare in order.
    assert!(
        compare(
            &rec(r#"{"value": {"a": 1, "b": 2}}"#),
            &ok(r#"{"b": 2, "a": 1}"#),
            false
        )
        .is_some()
    );
    // Errors compare by reason and message; syntax errors by kind.
    let err = rec(r#"{"error": "Expression.Error: x"}"#);
    assert_eq!(
        compare(&err, &Outcome::Error("Expression.Error: x".into()), false),
        None
    );
    assert!(compare(&err, &Outcome::Error("DataFormat.Error: x".into()), false).is_some());
    let syntax = rec(r#"{"error": "Expression.Error: [1,27-1,32] Token Literal expected."}"#);
    assert_eq!(compare(&syntax, &Outcome::Syntax("…".into()), false), None);
    assert!(compare(&syntax, &ok("1"), false).is_some());
    assert!(is_desktop_syntax_error(
        "Expression.Error: [1,1-1,2] Token Eof expected."
    ));
    assert!(!is_desktop_syntax_error("Expression.Error: [a] b"));
    // A rewritten case may move the position of an error, nothing else.
    let unknown = rec(r#"{"error": "Expression.Error: [1,9-1,19] The name 'x' doesn't exist."}"#);
    let moved = Outcome::Error("Expression.Error: [2,9-2,19] The name 'x' doesn't exist.".into());
    assert!(compare(&unknown, &moved, false).is_some());
    assert_eq!(compare(&unknown, &moved, true), None);
    let other = Outcome::Error("Expression.Error: [2,9-2,19] The name 'y' doesn't exist.".into());
    assert!(compare(&unknown, &other, true).is_some());
}
