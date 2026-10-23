//! Evaluation errors.

use pbi_engine::EngineError;

use crate::syntax::ParseError;

/// Maximum nesting of evaluation steps (AST levels plus measure → measure
/// calls). Deeper evaluations fail with [`DaxError::DepthExceeded`].
pub const MAX_EVAL_DEPTH: usize = 512;

/// An error raised while parsing or evaluating DAX. The `Display` text is
/// meant for end users (the UI shows "⚠ + message + DAX text").
#[derive(Debug, thiserror::Error)]
pub enum DaxError {
    /// The expression does not parse. `line`/`col` are 1-based.
    #[error("DAX syntax error at line {line}, column {col}: {message}")]
    Parse {
        message: String,
        line: usize,
        col: usize,
    },

    #[error("Cannot find table '{name}'.")]
    UnknownTable { name: String },

    /// `name` is the full reference, e.g. `'Web Sessions'[Device]`.
    #[error("Cannot find column {name}.")]
    UnknownColumn { name: String },

    #[error("Cannot find measure or column [{name}].")]
    UnknownMeasure { name: String },

    #[error("The DAX function {name} is not supported yet.")]
    UnsupportedFunction { name: String },

    /// A language feature other than a function (e.g. `DEFINE COLUMN`).
    #[error("{feature} is not supported yet.")]
    Unsupported { feature: String },

    /// Type mismatch or impossible conversion.
    #[error("{message}")]
    Type { message: String },

    /// Wrong number or kind of arguments.
    #[error("{function}: {message}")]
    Argument { function: String, message: String },

    /// Any other evaluation error (e.g. a column used without row context).
    #[error("{message}")]
    Evaluation { message: String },

    /// Measures (or calculated objects) that reference each other.
    #[error("A circular dependency was detected: {}.", chain.join(" → "))]
    CircularDependency { chain: Vec<String> },

    #[error("The expression is nested too deeply (more than {MAX_EVAL_DEPTH} evaluation levels).")]
    DepthExceeded,

    #[error("The evaluation was cancelled.")]
    Cancelled,

    /// Storage-engine failure (boxed to keep results small).
    #[error("Storage engine error: {0}")]
    Engine(Box<EngineError>),
}

impl DaxError {
    pub fn type_error(message: impl Into<String>) -> Self {
        DaxError::Type {
            message: message.into(),
        }
    }

    pub fn argument(function: &str, message: impl Into<String>) -> Self {
        DaxError::Argument {
            function: function.to_owned(),
            message: message.into(),
        }
    }

    pub fn evaluation(message: impl Into<String>) -> Self {
        DaxError::Evaluation {
            message: message.into(),
        }
    }

    pub fn unknown_column(table: &str, column: &str) -> Self {
        DaxError::UnknownColumn {
            name: format!("{}[{column}]", pbi_model::dax_table_ref(table)),
        }
    }

    /// Converts a parse error, computing the line and column in `src`.
    pub fn from_parse(err: &ParseError, src: &str) -> Self {
        let (line, col) = err.line_col(src);
        DaxError::Parse {
            message: err.to_string(),
            line,
            col,
        }
    }

    /// `true` when the expression uses something this engine does not
    /// implement yet (as opposed to an error in the expression or data).
    pub fn is_unsupported(&self) -> bool {
        matches!(
            self,
            DaxError::UnsupportedFunction { .. } | DaxError::Unsupported { .. }
        )
    }

    /// Errors `IFERROR`/`ISERROR` must not swallow.
    pub(crate) fn is_fatal(&self) -> bool {
        matches!(
            self,
            DaxError::Cancelled | DaxError::DepthExceeded | DaxError::CircularDependency { .. }
        )
    }
}

impl From<EngineError> for DaxError {
    fn from(err: EngineError) -> Self {
        match err {
            EngineError::TableNotFound(name) => DaxError::UnknownTable { name },
            EngineError::ColumnNotFound { table, column } => {
                DaxError::unknown_column(&table, &column)
            }
            EngineError::Cancelled => DaxError::Cancelled,
            EngineError::IncomparableTypes { left, right } => DaxError::Type {
                message: format!(
                    "DAX comparison operations do not support comparing values of type {} with values of type {}.",
                    dax_type_name(left),
                    dax_type_name(right)
                ),
            },
            EngineError::UnsupportedAggregation {
                agg,
                table,
                column,
                data_type,
            } => DaxError::Argument {
                function: agg.to_uppercase(),
                message: format!(
                    "cannot aggregate column {}[{column}] of type {data_type}",
                    pbi_model::dax_table_ref(&table)
                ),
            },
            other => DaxError::Engine(Box::new(other)),
        }
    }
}

/// DAX spelling of an engine type name.
pub(crate) fn dax_type_name(engine_name: &str) -> &str {
    match engine_name {
        "String" => "Text",
        "Boolean" => "True/False",
        "Double" => "Number",
        "Decimal" => "Currency",
        "DateTime" => "Date",
        other => other,
    }
}

/// Evaluation result type.
pub type Result<T, E = DaxError> = std::result::Result<T, E>;
