//! What M text can refer to besides its own bindings and the standard library: the other queries,
//! parameters and loaded tables of its model.

use std::collections::HashMap;

use crate::ast::{ExprKind, NullablePrimitiveType};

/// What an [`EnvironmentEntry`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvironmentKind {
    /// A query or other M expression of the model (a function query too).
    Query,
    /// A Power Query parameter (`… meta [IsParameterQuery = true]`).
    Parameter,
    /// A table loaded by an M query: the name refers to the table's query.
    Table,
}

/// A name of the model that M text can refer to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentEntry {
    pub name: String,
    pub kind: EnvironmentKind,
    /// The M text: a query's expression, a parameter's value, a table's partition query (empty
    /// when unknown).
    pub expression: String,
    /// A parameter's type from its metadata (`Text`, `Number`, …).
    pub type_name: Option<String>,
    /// The model's description of the query, parameter or table.
    pub description: Option<String>,
}

impl EnvironmentEntry {
    pub fn new(
        name: impl Into<String>,
        kind: EnvironmentKind,
        expression: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind,
            expression: expression.into(),
            type_name: None,
            description: None,
        }
    }

    #[must_use]
    pub fn with_type_name(mut self, type_name: Option<String>) -> Self {
        self.type_name = type_name;
        self
    }

    #[must_use]
    pub fn with_description(mut self, description: Option<String>) -> Self {
        self.description = description.filter(|d| !d.trim().is_empty());
        self
    }
}

/// The names a query can refer to besides its own: the other queries, parameters and loaded
/// tables of its model.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    /// `None`: not known (no model).
    entries: Option<Vec<EnvironmentEntry>>,
    index: HashMap<String, usize>,
    /// The parameters and return type of entries that are function literals.
    functions: Vec<Option<FunctionShape>>,
}

impl Environment {
    /// Unknown names around the text (no model): a plain unknown name may be another query and
    /// is not reported, while a misspelled library member (`Table.RowCnt`) still is.
    pub fn unknown() -> Self {
        Self::default()
    }

    /// Exactly `names` (queries, parameters and loaded tables of the model) besides the library,
    /// as queries whose text is not known.
    pub fn with_names<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::with_entries(
            names
                .into_iter()
                .map(|n| EnvironmentEntry::new(n, EnvironmentKind::Query, "")),
        )
    }

    /// Exactly `entries` besides the library (the first of two entries with the same name wins).
    pub fn with_entries(entries: impl IntoIterator<Item = EnvironmentEntry>) -> Self {
        let mut list = Vec::new();
        let mut index = HashMap::new();
        for e in entries {
            if !index.contains_key(&e.name) {
                index.insert(e.name.clone(), list.len());
                list.push(e);
            }
        }
        let functions = list
            .iter()
            .map(|e| match e.kind {
                EnvironmentKind::Query => function_shape(&e.expression),
                EnvironmentKind::Parameter | EnvironmentKind::Table => None,
            })
            .collect();
        Self {
            entries: Some(list),
            index,
            functions,
        }
    }

    /// The entries, in the order given (empty when unknown).
    pub fn entries(&self) -> &[EnvironmentEntry] {
        self.entries.as_deref().unwrap_or(&[])
    }

    /// The entry named exactly `name`.
    pub fn entry(&self, name: &str) -> Option<&EnvironmentEntry> {
        let i = *self.index.get(name)?;
        self.entries().get(i)
    }

    /// The parameters and return type of the query `name` when it is a function literal.
    pub(crate) fn function(&self, name: &str) -> Option<&FunctionShape> {
        let i = *self.index.get(name)?;
        self.functions.get(i)?.as_ref()
    }

    /// The function shape of entry `i` of [`Self::entries`].
    pub(crate) fn function_at(&self, i: usize) -> Option<&FunctionShape> {
        self.functions.get(i)?.as_ref()
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// Whether the model's names are known.
    pub(crate) fn is_known(&self) -> bool {
        self.entries.is_some()
    }
}

/// A parameter of a function literal or library function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParamShape {
    pub name: String,
    pub optional: bool,
    /// `nullable text`; `any` when the text gives no type.
    pub ty: String,
}

/// The parameters and return type of a function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FunctionShape {
    pub params: Vec<ParamShape>,
    pub returns: String,
}

/// Longest M text parsed to find a function's parameters.
const MAX_FUNCTION_TEXT: usize = 1 << 20;

/// The shape of `text` when it is a function literal (`(x as text) => …`, `each …`).
pub(crate) fn function_shape(text: &str) -> Option<FunctionShape> {
    let trimmed = text.trim_start();
    if text.len() > MAX_FUNCTION_TEXT || !(trimmed.starts_with('(') || trimmed.starts_with("each"))
    {
        return None;
    }
    let expr = crate::parse_expression(text).ok()?;
    match &expr.unparenthesized().kind {
        ExprKind::Function {
            params,
            return_type,
            ..
        } => Some(FunctionShape {
            params: params
                .iter()
                .map(|p| ParamShape {
                    name: p.name.text.clone(),
                    optional: p.optional,
                    ty: p.ty.map_or_else(|| "any".to_owned(), type_text),
                })
                .collect(),
            returns: return_type.map_or_else(|| "any".to_owned(), type_text),
        }),
        ExprKind::Each(_) => Some(FunctionShape {
            params: vec![ParamShape {
                name: "_".to_owned(),
                optional: false,
                ty: "any".to_owned(),
            }],
            returns: "any".to_owned(),
        }),
        _ => None,
    }
}

fn type_text(t: NullablePrimitiveType) -> String {
    if t.nullable {
        format!("nullable {}", t.primitive.name())
    } else {
        t.primitive.name().to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_index_and_function_shapes() {
        let env = Environment::with_entries([
            EnvironmentEntry::new(
                "Clean",
                EnvironmentKind::Query,
                "(t as table, optional n as nullable number) as table => t",
            ),
            EnvironmentEntry::new("Twice", EnvironmentKind::Query, "each _ * 2"),
            EnvironmentEntry::new("Orders", EnvironmentKind::Table, "let a = 1 in a"),
            EnvironmentEntry::new("Clean", EnvironmentKind::Parameter, "\"x\""),
        ]);
        assert_eq!(env.entries().len(), 3);
        assert_eq!(
            env.entry("Clean").map(|e| e.kind),
            Some(EnvironmentKind::Query)
        );
        let clean = env.function("Clean").map(|f| {
            f.params
                .iter()
                .map(|p| format!("{} {} {}", p.optional, p.name, p.ty))
                .chain([f.returns.clone()])
                .collect::<Vec<_>>()
        });
        assert_eq!(
            clean,
            Some(vec![
                "false t table".to_owned(),
                "true n nullable number".to_owned(),
                "table".to_owned()
            ])
        );
        assert_eq!(env.function("Twice").map(|f| f.params.len()), Some(1));
        assert!(env.function("Orders").is_none());
        assert!(Environment::unknown().entries().is_empty());
        assert!(!Environment::unknown().is_known());
        assert!(Environment::with_names(["a"]).contains("a"));
    }
}
