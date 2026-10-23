//! Turning a model's answer into a typed result: structured JSON, a fenced
//! ```json block, or the first JSON object in the text; then deserialized
//! into the capability's output type (the type its schema is generated
//! from).

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::capability::Capability;
use crate::error::{AiError, truncate};

/// Longest raw answer kept in an [`AiError::InvalidOutput`].
const MAX_RAW_CHARS: usize = 1500;

/// The JSON value in `text`: the whole text, else a fenced block (` ```json `
/// first), else the first balanced `{…}`.
#[must_use]
pub fn extract_json(text: &str) -> Option<Value> {
    let trimmed = text.trim().trim_start_matches('\u{feff}');
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Some(v);
    }
    let blocks = fenced_blocks(trimmed);
    let tagged = blocks
        .iter()
        .filter(|(lang, _)| lang.eq_ignore_ascii_case("json"));
    let untagged = blocks
        .iter()
        .filter(|(lang, _)| !lang.eq_ignore_ascii_case("json"));
    for (_, body) in tagged.chain(untagged) {
        if let Ok(v) = serde_json::from_str::<Value>(body.trim()) {
            return Some(v);
        }
    }
    first_object(trimmed).and_then(|s| serde_json::from_str(s).ok())
}

/// ```` ```lang\n…\n``` ```` blocks: `(lang, body)`.
fn fenced_blocks(text: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("```") {
        let after = rest.get(start + 3..).unwrap_or_default();
        let (lang, body_start) = match after.find('\n') {
            Some(nl) => (after.get(..nl).unwrap_or_default().trim(), nl + 1),
            None => break,
        };
        let body_all = after.get(body_start..).unwrap_or_default();
        let Some(end) = body_all.find("```") else {
            break;
        };
        out.push((lang, body_all.get(..end).unwrap_or_default()));
        rest = body_all.get(end + 3..).unwrap_or_default();
    }
    out
}

/// The first balanced `{…}` (string-aware).
fn first_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, c) in text.get(start..)?.char_indices() {
        if in_string {
            match c {
                _ if escaped => escaped = false,
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return text.get(start..=start + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parses the answer `text` of `capability` into `T`.
///
/// # Errors
/// [`AiError::InvalidOutput`] with the reason and the raw text (truncated;
/// redact it before calling if it may contain secrets).
pub fn parse_output<T: DeserializeOwned>(capability: Capability, text: &str) -> Result<T, AiError> {
    let invalid = |message: String| AiError::InvalidOutput {
        capability,
        message,
        raw: truncate(text.trim(), MAX_RAW_CHARS),
    };
    let value = extract_json(text).ok_or_else(|| invalid("no JSON object found".into()))?;
    if !value.is_object() {
        return Err(invalid("the JSON is not an object".into()));
    }
    serde_json::from_value(value).map_err(|e| invalid(format!("does not match the schema: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::DaxGenerateOutput;
    use serde_json::json;

    #[test]
    fn json_is_found_plain_fenced_or_embedded() {
        assert_eq!(extract_json(" {\"a\": 1} "), Some(json!({"a": 1})));
        assert_eq!(
            extract_json("Here you go:\n```json\n{\"a\": 2}\n```\nDone."),
            Some(json!({"a": 2}))
        );
        assert_eq!(
            extract_json("```\nnot json\n```\n```\n{\"a\": 3}\n```"),
            Some(json!({"a": 3}))
        );
        assert_eq!(
            extract_json("Sure! {\"a\": \"}{\", \"b\": {\"c\": 4}} trailing"),
            Some(json!({"a": "}{", "b": {"c": 4}}))
        );
        assert_eq!(extract_json("no json here"), None);
        assert_eq!(extract_json("{ unbalanced"), None);
    }

    #[test]
    fn invalid_answers_carry_the_raw_text() {
        let e = parse_output::<DaxGenerateOutput>(Capability::DaxGenerate, "I cannot help")
            .unwrap_err();
        let text = e.to_string();
        assert!(
            text.contains("no JSON object") && text.contains("I cannot help"),
            "{text}"
        );
        let e = parse_output::<DaxGenerateOutput>(Capability::DaxGenerate, "{\"expression\": 42}")
            .unwrap_err();
        assert!(e.to_string().contains("does not match the schema"), "{e}");
        let long = format!("{{\"expression\": {}}}", "9".repeat(5000));
        let e = parse_output::<DaxGenerateOutput>(Capability::DaxGenerate, &long).unwrap_err();
        assert!(e.to_string().len() < 2500);
        let ok: DaxGenerateOutput = parse_output(
            Capability::DaxGenerate,
            "```json\n{\"name\": \"X\", \"expression\": \"1\"}\n```",
        )
        .unwrap();
        assert_eq!(ok.expression, "1");
    }
}
