//! Ordered batches of commands sharing one workspace, with `${name.field}`
//! interpolation from earlier results (`dashcraft run --batch`,
//! `commands.batch`, and the MCP `command_batch` tool to come).
//!
//! A step is `{ "id": "doc.open", "params": {…}, "as": "d" }`. Before a step
//! runs, every string in its `params` is interpolated:
//!
//! - a string that is exactly `${name.path}` becomes the referenced JSON
//!   value (numbers stay numbers, objects stay objects);
//! - `${name.path}` inside a longer string is replaced by the value's text
//!   (strings as they are, other values as JSON);
//! - `$${` is a literal `${`.
//!
//! `name` is the `as` of an earlier step; `path` is a dotted path into its
//! result (`d.docId`, `p.pages.0.name`; array items by index). Unknown
//! names or paths fail the step with `invalidParams`.
//!
//! Batches are **not atomic**: steps that ran stay applied when a later step
//! fails. With `stopOnError` (the default) the batch stops at the first
//! failing step; the remaining steps are not run and not listed.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::ctx::Ctx;
use crate::error::OpError;
use crate::registry::Registry;
use crate::workspace::Workspace;

/// Longest interpolated string (characters).
const MAX_INTERPOLATED_CHARS: usize = 1 << 20;
/// Most steps in one batch.
pub const MAX_STEPS: usize = 1000;

/// One step of a batch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BatchStep {
    /// Command id.
    pub id: String,
    /// Parameters; strings may reference earlier results as
    /// `${name.field}`.
    #[serde(default)]
    pub params: Value,
    /// Name this step's result for later steps.
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
}

/// Result of one step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BatchStepResult {
    /// 0-based position in the batch.
    pub index: usize,
    pub id: String,
    #[serde(default, rename = "as", skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// The command succeeded (its result may still report data problems,
    /// e.g. `"ok": false`).
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<OpError>,
}

/// Runs `steps` in order on `ws`. `vars` are names available to every
/// step (e.g. a document opened beforehand).
pub async fn run_batch(
    registry: &Registry,
    ws: &Workspace,
    steps: &[BatchStep],
    stop_on_error: bool,
    vars: Map<String, Value>,
    ctx: &Ctx,
) -> Vec<BatchStepResult> {
    let mut vars = vars;
    let mut out = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        let result = match interpolate(&step.params, &vars) {
            Ok(params) => registry.execute(ws, &step.id, params, ctx).await,
            Err(e) => Err(e),
        };
        let failed = result.is_err();
        let (result, error) = match result {
            Ok(v) => {
                if let Some(name) = &step.alias {
                    vars.insert(name.clone(), v.clone());
                }
                (Some(v), None)
            }
            Err(e) => (None, Some(e)),
        };
        out.push(BatchStepResult {
            index,
            id: step.id.clone(),
            alias: step.alias.clone(),
            ok: !failed,
            result,
            error,
        });
        if failed && (stop_on_error || ctx.is_cancelled()) {
            break;
        }
    }
    out
}

/// Interpolates every string of `params` (see the [module docs](self)).
///
/// # Errors
/// `invalidParams` for an unknown name or path, or an unterminated `${`.
pub fn interpolate(params: &Value, vars: &Map<String, Value>) -> Result<Value, OpError> {
    match params {
        Value::String(s) => interpolate_str(s, vars),
        Value::Array(items) => items
            .iter()
            .map(|v| interpolate(v, vars))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| Ok((k.clone(), interpolate(v, vars)?)))
            .collect::<Result<Map<_, _>, OpError>>()
            .map(Value::Object),
        other => Ok(other.clone()),
    }
}

fn interpolate_str(s: &str, vars: &Map<String, Value>) -> Result<Value, OpError> {
    if !s.contains("${") {
        return Ok(Value::String(s.to_owned()));
    }
    // A whole-string reference keeps the value's type.
    if let Some(inner) = s.strip_prefix("${").and_then(|r| r.strip_suffix('}'))
        && !inner.contains('}')
        && !s.starts_with("$${")
    {
        return lookup(inner, vars).cloned();
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('$') {
        out.push_str(rest.get(..i).unwrap_or_default());
        let tail = rest.get(i..).unwrap_or_default();
        if let Some(after) = tail.strip_prefix("$${") {
            out.push_str("${");
            rest = after;
        } else if let Some(after) = tail.strip_prefix("${") {
            let end = after
                .find('}')
                .ok_or_else(|| OpError::invalid_params(format!("unterminated `${{` in {s:?}")))?;
            let value = lookup(after.get(..end).unwrap_or_default(), vars)?;
            match value {
                Value::String(v) => out.push_str(v),
                other => out.push_str(&other.to_string()),
            }
            rest = after.get(end + 1..).unwrap_or_default();
        } else {
            out.push('$');
            rest = tail.get(1..).unwrap_or_default();
        }
        if out.chars().count() > MAX_INTERPOLATED_CHARS {
            return Err(OpError::invalid_params(
                "an interpolated string is too long",
            ));
        }
    }
    out.push_str(rest);
    Ok(Value::String(out))
}

fn lookup<'a>(path: &str, vars: &'a Map<String, Value>) -> Result<&'a Value, OpError> {
    let path = path.trim();
    let mut parts = path.split('.');
    let name = parts.next().unwrap_or_default();
    let mut value = vars.get(name).ok_or_else(|| {
        OpError::invalid_params(format!(
            "`${{{path}}}`: no earlier step is named '{name}' (name steps with \"as\")"
        ))
    })?;
    for part in parts {
        value = match value {
            Value::Object(map) => map.get(part),
            Value::Array(items) => part.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        }
        .ok_or_else(|| {
            OpError::invalid_params(format!("`${{{path}}}`: '{part}' is not in the result"))
        })?;
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vars() -> Map<String, Value> {
        let mut m = Map::new();
        m.insert(
            "d".into(),
            json!({"docId": "d1", "n": 3, "pages": [{"name": "p1"}]}),
        );
        m
    }

    #[test]
    fn whole_string_references_keep_their_type() {
        let v = interpolate(&json!({"docId": "${d.docId}", "limit": "${d.n}"}), &vars());
        assert_eq!(v.ok(), Some(json!({"docId": "d1", "limit": 3})));
        let v = interpolate(&json!("${d.pages.0.name}"), &vars());
        assert_eq!(v.ok(), Some(json!("p1")));
    }

    #[test]
    fn embedded_references_are_text() {
        let v = interpolate(
            &json!(["doc ${d.docId} has ${d.n} pages", "$${d.n}", "$5"]),
            &vars(),
        );
        assert_eq!(v.ok(), Some(json!(["doc d1 has 3 pages", "${d.n}", "$5"])));
    }

    #[test]
    fn unknown_references_fail() {
        for bad in ["${x.docId}", "${d.missing}", "a ${d.docId"] {
            let e = interpolate(&json!(bad), &vars()).err().map(|e| e.code);
            assert_eq!(e, Some(crate::OpErrorCode::InvalidParams), "{bad}");
        }
    }
}
