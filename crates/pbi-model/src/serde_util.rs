//! `skip_serializing_if` helpers that keep the JSON form compact.

pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

pub(crate) fn is_true(value: &bool) -> bool {
    *value
}

pub(crate) fn is_zero(value: &i64) -> bool {
    *value == 0
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn is_null(value: &serde_json::Value) -> bool {
    value.is_null()
}
