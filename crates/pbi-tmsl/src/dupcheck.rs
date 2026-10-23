//! Duplicate-key detection. `serde_json` keeps the last value of a
//! duplicated key silently; this pass reports them so nothing is dropped
//! without a diagnostic.

use std::collections::HashSet;
use std::fmt;

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};

use crate::obj::child_pointer;

/// Returns the JSON pointers of duplicated keys. Fails on malformed JSON.
pub(crate) fn find_duplicate_keys(text: &str) -> Result<Vec<String>, serde_json::Error> {
    let mut duplicates = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(text);
    DupCheck {
        pointer: String::new(),
        duplicates: &mut duplicates,
    }
    .deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(duplicates)
}

struct DupCheck<'a> {
    pointer: String,
    duplicates: &'a mut Vec<String>,
}

impl<'de> DeserializeSeed<'de> for DupCheck<'_> {
    type Value = ();

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for DupCheck<'_> {
    type Value = ();

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_bool<E>(self, _: bool) -> Result<(), E> {
        Ok(())
    }

    fn visit_i64<E>(self, _: i64) -> Result<(), E> {
        Ok(())
    }

    fn visit_u64<E>(self, _: u64) -> Result<(), E> {
        Ok(())
    }

    fn visit_f64<E>(self, _: f64) -> Result<(), E> {
        Ok(())
    }

    fn visit_str<E>(self, _: &str) -> Result<(), E> {
        Ok(())
    }

    fn visit_unit<E>(self) -> Result<(), E> {
        Ok(())
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        let DupCheck {
            pointer,
            duplicates,
        } = self;
        let mut index = 0usize;
        loop {
            let seed = DupCheck {
                pointer: format!("{pointer}/{index}"),
                duplicates: &mut *duplicates,
            };
            if seq.next_element_seed(seed)?.is_none() {
                return Ok(());
            }
            index += 1;
        }
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        let DupCheck {
            pointer,
            duplicates,
        } = self;
        let mut seen = HashSet::new();
        while let Some(key) = map.next_key::<String>()? {
            let child = child_pointer(&pointer, &key);
            if !seen.insert(key) {
                duplicates.push(child.clone());
            }
            map.next_value_seed(DupCheck {
                pointer: child,
                duplicates: &mut *duplicates,
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_duplicates_at_any_depth() {
        let dups = find_duplicate_keys(
            r#"{"a": 1, "b": [{"x": 1.50, "x": 2}], "a": {"c/d": 1, "c/d": 2}}"#,
        )
        .unwrap();
        assert_eq!(dups, ["/b/0/x", "/a", "/a/c~1d"]);
        assert!(find_duplicate_keys(r#"{"a": 1"#).is_err());
        assert!(find_duplicate_keys(r#"{} x"#).is_err());
    }
}
