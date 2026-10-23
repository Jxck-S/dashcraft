//! Enumerations used by the semantic model. All of them parse leniently and
//! keep unknown values (see [`crate::macros`]).

use crate::macros::string_enum;

string_enum! {
    /// Column data type (TOM `DataType`). Serialized with the TMDL spelling.
    #[derive(Default)]
    pub enum DataType {
        String = "string" | "text" | "wstr" | "str",
        Int64 = "int64" | "integer" | "int" | "wholeNumber" | "i8",
        Double = "double" | "number" | "float" | "r8" | "decimalNumber",
        Decimal = "decimal" | "currency" | "fixedDecimalNumber" | "cy",
        DateTime = "dateTime" | "date" | "time" | "dateTimeZone",
        Boolean = "boolean" | "bool" | "logical" | "trueFalse",
        Binary = "binary",
        Variant = "variant",
        /// TOM `Automatic`: type not determined yet (e.g. new calculated column).
        #[default]
        Automatic = "automatic",
    }
}

impl DataType {
    /// `Int64`, `Double` or `Decimal`.
    pub fn is_numeric(&self) -> bool {
        matches!(self, DataType::Int64 | DataType::Double | DataType::Decimal)
    }
}

string_enum! {
    /// Default aggregation of a column (TOM `SummarizeBy`).
    pub enum SummarizeBy {
        Default = "default",
        None = "none" | "doNotSummarize" | "dontSummarize",
        Sum = "sum",
        Min = "min",
        Max = "max",
        Count = "count",
        Average = "average" | "avg",
        DistinctCount = "distinctCount",
    }
}

string_enum! {
    /// Storage mode of a partition (TOM `ModeType`).
    #[derive(Default)]
    pub enum PartitionMode {
        Import = "import",
        DirectQuery = "directQuery",
        Dual = "dual",
        /// Inherit the model's default mode.
        #[default]
        Default = "default",
        DirectLake = "directLake",
    }
}

string_enum! {
    /// Cardinality of one relationship end.
    pub enum Cardinality {
        One = "one",
        Many = "many",
    }
}

string_enum! {
    /// Relationship cross-filter direction (TOM `CrossFilteringBehavior`).
    #[derive(Default)]
    pub enum CrossFilter {
        #[default]
        OneDirection = "oneDirection" | "single",
        BothDirections = "bothDirections" | "both",
        Automatic = "automatic",
    }
}

string_enum! {
    /// Relationship security filtering behaviour (RLS propagation).
    #[derive(Default)]
    pub enum SecurityFilteringBehavior {
        #[default]
        OneDirection = "oneDirection" | "single",
        BothDirections = "bothDirections" | "both",
        None = "none",
    }
}

string_enum! {
    /// Language of a named (shared) expression.
    #[derive(Default)]
    pub enum ExpressionKind {
        #[default]
        M = "m" | "powerQuery",
        Dax = "dax",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_type_parses_tmdl_tmsl_and_ui_spellings() {
        let cases = [
            ("string", DataType::String),
            ("String", DataType::String),
            ("int64", DataType::Int64),
            ("Whole Number", DataType::Int64),
            ("double", DataType::Double),
            ("decimal", DataType::Decimal),
            ("Fixed Decimal Number", DataType::Decimal),
            ("dateTime", DataType::DateTime),
            ("DATETIME", DataType::DateTime),
            ("boolean", DataType::Boolean),
            ("binary", DataType::Binary),
            ("variant", DataType::Variant),
            ("automatic", DataType::Automatic),
            (" int64 ", DataType::Int64),
        ];
        for (input, expected) in cases {
            assert_eq!(DataType::parse(input), expected, "input {input:?}");
        }
        let unknown = DataType::parse("geography");
        assert_eq!(unknown, DataType::Unknown("geography".into()));
        assert_eq!(unknown.as_str(), "geography");
        assert_eq!(DataType::DateTime.to_string(), "dateTime");
    }

    #[test]
    fn string_enums_serialize_as_canonical_strings() {
        let json = serde_json::to_string(&vec![DataType::DateTime, DataType::Unknown("x".into())])
            .unwrap();
        assert_eq!(json, r#"["dateTime","x"]"#);
        let back: Vec<DataType> = serde_json::from_str(r#"["DateTime","int64","x"]"#).unwrap();
        assert_eq!(
            back,
            vec![
                DataType::DateTime,
                DataType::Int64,
                DataType::Unknown("x".into())
            ]
        );
        let cf: CrossFilter = serde_json::from_str(r#""bothDirections""#).unwrap();
        assert_eq!(cf, CrossFilter::BothDirections);
        assert_eq!(
            "doNotSummarize".parse::<SummarizeBy>(),
            Ok(SummarizeBy::None)
        );
    }
}
