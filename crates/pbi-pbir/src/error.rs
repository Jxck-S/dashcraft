//! Errors and diagnostic codes.

/// Hard failures. Problems inside an otherwise readable report become
/// [`pbi_model::Diagnostic`]s instead (see [`codes`]).
#[derive(Debug, thiserror::Error)]
pub enum PbirError {
    /// The source has neither `definition/report.json` nor any page.
    #[error("not a PBIR report folder: {0}")]
    NotPbir(String),
    /// A legacy layout whose bytes are not text in a supported encoding.
    #[error("invalid text encoding: {0}")]
    Encoding(String),
    /// A JSON document that does not parse or has the wrong shape.
    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: String,
        #[source]
        source: serde_json::Error,
    },
    /// A legacy layout without the expected structure.
    #[error("invalid legacy layout: {0}")]
    Legacy(String),
}

/// Stable diagnostic codes emitted by this crate.
pub mod codes {
    /// A file is not valid JSON or does not match its expected shape; it is
    /// kept verbatim and skipped.
    pub const INVALID_FILE: &str = "PBIR001";
    /// An expected file is missing (e.g. a page listed in `pageOrder`).
    pub const MISSING_FILE: &str = "PBIR002";
    /// A `SourceRef.Source` alias is not declared in `From`.
    pub const UNRESOLVED_ALIAS: &str = "PBIR003";
    /// A page exists on disk but is not listed in `pageOrder`.
    pub const UNLISTED_PAGE: &str = "PBIR004";
    /// A legacy stringified JSON property (`config`, `filters`, …) is
    /// invalid.
    pub const INVALID_EMBEDDED_JSON: &str = "PBIR005";
    /// A legacy projection `queryRef` has no matching `prototypeQuery`
    /// select item.
    pub const UNRESOLVED_QUERY_REF: &str = "PBIR006";
    /// A theme referenced by `report.json` is missing or invalid.
    pub const THEME: &str = "PBIR007";
}
