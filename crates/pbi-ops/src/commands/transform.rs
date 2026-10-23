//! `text.parsePeople`, `text.previewPeople`.

use std::collections::HashSet;
use std::sync::Arc;

use arrow::array::{Array, ArrayRef, AsArray};
use pbi_session::SessionError;
use pbi_session::transforms::people::{People, parse_people, text_array};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{BASE, bounded, loaded_doc};
use crate::command::{Category, Command, CommandMeta};
use crate::ctx::Ctx;
use crate::error::OpError;
use crate::resolve;
use crate::workspace::Workspace;

/// Longest input (characters).
const MAX_TEXT_CHARS: usize = 1 << 20;
/// Largest `limit` of `text.previewPeople`.
pub const MAX_PEOPLE_SAMPLES: usize = 200;
/// Longest raw sample text in a people preview (characters).
const MAX_SAMPLE_CHARS: usize = 2_000;

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", text.get(..cut).unwrap_or_default()),
        None => text.to_owned(),
    }
}

/// `text.parsePeople`.
#[derive(Debug)]
pub struct TextParsePeople;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ParsePeopleParams {
    /// One exported people-picker cell: a Python-style list of dicts, e.g.
    /// `[{'firstName': 'Ada', 'lastName': 'Lovelace'}]`.
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ParsePeopleOut {
    /// `names` (a people list), `empty` (blank or an empty list) or
    /// `unparsed` (not a people list; `display` is the trimmed text).
    pub outcome: String,
    /// `First Last` per person.
    pub names: Vec<String>,
    /// What the "Parse people field" column shows: names joined with `; `.
    pub display: Option<String>,
}

impl Command for TextParsePeople {
    type Params = ParsePeopleParams;
    type Output = ParsePeopleOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "text.parsePeople",
            title: "Parse a people field",
            summary: "Parses one people-picker cell (a Python-style list of dicts) into `First Last` names (the \"Parse people field\" transform).",
            category: Category::Transform,
            examples: &[(
                "Two people",
                r#"{"text": "[{'firstName': 'Ada', 'lastName': 'Lovelace'}, {'firstName': 'Alan', 'lastName': 'Turing'}]"}"#,
            )],
            ..BASE
        }
    }

    async fn run(_ws: &Workspace, p: Self::Params, _ctx: &Ctx) -> Result<Self::Output, OpError> {
        if p.text.chars().count() > MAX_TEXT_CHARS {
            return Err(OpError::invalid_params(format!(
                "text is longer than {MAX_TEXT_CHARS} characters"
            )));
        }
        let people = parse_people(&p.text);
        let display = people.display();
        let (outcome, names) = match people {
            People::Empty => ("empty", Vec::new()),
            People::Names(names) => ("names", names),
            People::Unparsed(_) => ("unparsed", Vec::new()),
        };
        Ok(ParsePeopleOut {
            outcome: outcome.to_owned(),
            names,
            display,
        })
    }
}

// ---------------------------------------------------------------------------

/// `text.previewPeople`.
#[derive(Debug)]
pub struct TextPreviewPeople;

fn default_samples() -> usize {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewPeopleParams {
    /// The document id from `doc.open` (`d1`, …).
    pub doc_id: String,
    /// Table name (case-insensitive).
    pub table: String,
    /// A text column of the table (case-insensitive).
    pub column: String,
    /// Distinct samples returned (default 50, at most 200).
    #[serde(default = "default_samples")]
    pub limit: usize,
}

/// One distinct value of a people preview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PeopleSampleOut {
    /// The cell text (cut at 2000 characters).
    pub raw: String,
    /// What the parsed column shows (`First Last; …`, or the trimmed text).
    pub parsed: Option<String>,
    /// `names` or `unparsed`.
    pub outcome: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PeoplePreviewOut {
    /// Table name (model spelling).
    pub table: String,
    /// Column name (stored spelling).
    pub column: String,
    /// The name `data.addDerivedColumn` should use: `<column> (parsed)`.
    pub new_column: String,
    pub rows: usize,
    /// Cells parsed into names.
    pub names: usize,
    /// Non-empty cells that are not a people list (shown trimmed).
    pub unparsed: usize,
    /// Blank cells and empty lists.
    pub empty: usize,
    /// Distinct non-empty values in storage order, with their parsed form.
    pub samples: Vec<PeopleSampleOut>,
}

impl Command for TextPreviewPeople {
    type Params = PreviewPeopleParams;
    type Output = PeoplePreviewOut;

    fn meta() -> CommandMeta {
        CommandMeta {
            id: "text.previewPeople",
            title: "Preview \"Parse people field\"",
            summary: "Parses every cell of a text column as a people list: outcome counts and distinct samples with their parsed form (before data.addDerivedColumn).",
            category: Category::Transform,
            examples: &[(
                "Account managers",
                r#"{"docId": "d1", "table": "Customers", "column": "Account Managers", "limit": 20}"#,
            )],
            ..BASE
        }
    }

    async fn run(ws: &Workspace, p: Self::Params, ctx: &Ctx) -> Result<Self::Output, OpError> {
        let limit = bounded("limit", p.limit, MAX_PEOPLE_SAMPLES)?;
        let entry = loaded_doc(ws, &p.doc_id, ctx).await?;
        let session = entry.session();
        let table = resolve::table_name(&session.model(), &p.table)?;
        let snapshot = session.snapshot();
        let engine = |e| OpError::from(SessionError::from(e));
        let stored = Arc::clone(snapshot.table(&table).map_err(engine)?);
        let column = stored
            .column_name(&p.column)
            .map_err(|e| engine(e).context(format!("'{table}'")))?
            .to_owned();
        let array = Arc::clone(stored.column(&column).map_err(engine)?);
        tokio::task::spawn_blocking(move || people_preview(&table, &column, &array, limit)).await?
    }
}

/// Outcome counts over the whole column plus the first `limit` distinct
/// non-empty samples.
///
/// # Errors
/// `query` when the column is not text.
pub fn people_preview(
    table: &str,
    column: &str,
    array: &ArrayRef,
    limit: usize,
) -> Result<PeoplePreviewOut, OpError> {
    let text = text_array(array).ok_or_else(|| {
        OpError::query(format!(
            "\"Parse people field\" needs a text column, not {}",
            array.data_type()
        ))
    })?;
    let strings = text.as_string::<i32>();
    let (mut names, mut unparsed, mut empty) = (0usize, 0usize, 0usize);
    let mut seen = HashSet::new();
    let mut samples = Vec::new();
    for value in strings.iter() {
        let parsed = value.map_or(People::Empty, parse_people);
        let outcome = match &parsed {
            People::Names(_) => {
                names += 1;
                "names"
            }
            People::Unparsed(_) => {
                unparsed += 1;
                "unparsed"
            }
            People::Empty => {
                empty += 1;
                continue;
            }
        };
        if let Some(raw) = value
            && samples.len() < limit
            && seen.insert(raw)
        {
            samples.push(PeopleSampleOut {
                raw: truncate(raw, MAX_SAMPLE_CHARS),
                parsed: parsed.display(),
                outcome: outcome.to_owned(),
            });
        }
    }
    Ok(PeoplePreviewOut {
        table: table.to_owned(),
        column: column.to_owned(),
        new_column: format!("{column} (parsed)"),
        rows: strings.len(),
        names,
        unparsed,
        empty,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use arrow::array::{Int64Array, StringArray};

    use super::*;

    #[test]
    fn people_previews_count_outcomes() {
        let array: ArrayRef = Arc::new(StringArray::from(vec![
            Some("[{'firstName': 'Ada', 'lastName': 'Lovelace'}]"),
            Some("[{'firstName': 'Ada', 'lastName': 'Lovelace'}]"),
            Some(
                r#"[{'firstName': 'Sean', 'lastName': "O'Neill"}, {'firstName': 'Alan', 'lastName': 'Turing'}]"#,
            ),
            Some("Grace Hopper"),
            Some("[]"),
            None,
        ]));
        let preview =
            people_preview("Owners", "Owner", &array, 10).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(preview.new_column, "Owner (parsed)");
        assert_eq!(
            (preview.rows, preview.names, preview.unparsed, preview.empty),
            (6, 3, 1, 2)
        );
        // Distinct non-empty samples, in storage order.
        let samples: Vec<_> = preview
            .samples
            .iter()
            .map(|s| (s.parsed.as_deref(), s.outcome.as_str()))
            .collect();
        assert_eq!(
            samples,
            [
                (Some("Ada Lovelace"), "names"),
                (Some("Sean O'Neill; Alan Turing"), "names"),
                (Some("Grace Hopper"), "unparsed"),
            ]
        );
        let one = people_preview("T", "C", &array, 1).map(|p| p.samples.len());
        assert_eq!(one.ok(), Some(1));

        let numbers: ArrayRef = Arc::new(Int64Array::from(vec![1, 2]));
        let err = people_preview("T", "C", &numbers, 10).err().map(|e| e.code);
        assert_eq!(err, Some(crate::OpErrorCode::Query));
    }
}
