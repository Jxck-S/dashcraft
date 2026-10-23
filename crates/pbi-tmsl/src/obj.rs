//! Generic object reader/writer that splits a JSON object into typed fields
//! and an `extra` [`PropertyBag`], and merges them back.
//!
//! # Extra-bag conventions
//!
//! - Keys the model does not type are stored verbatim as
//!   [`PropValue::Json`], in source order.
//! - **Shadows:** when the default encoding of a typed field would not
//!   reproduce the source JSON (explicit default value, text form, enum
//!   spelling, …), the original JSON is also stored under the same key. The
//!   writer emits a shadow only while it still decodes to the typed value;
//!   otherwise the typed value wins. Other writers must likewise let typed
//!   fields win over same-named `extra` entries.
//! - **Absent marker:** an empty [`PropValue::List`] means "the key was
//!   absent although the typed default would be written" (only `kind` of
//!   named expressions and `source` of partitions need it).

use std::cmp::Ordering;

use pbi_model::{Diagnostic, Origin, PropValue, PropertyBag, Severity};
use serde_json::{Map, Value};

use crate::codec::Codec;
use crate::error::TmslError;
use crate::format::KeyOrder;

/// Shared reader state.
#[derive(Debug, Default)]
pub(crate) struct ReadCx {
    pub diagnostics: Vec<Diagnostic>,
    pub source_path: Option<String>,
    /// Every typed object so far had its keys in [`KeyOrder::Alphabetical`]
    /// order.
    pub alphabetical: bool,
}

impl ReadCx {
    pub fn new(source_path: Option<String>) -> Self {
        Self {
            diagnostics: Vec::new(),
            source_path,
            alphabetical: true,
        }
    }

    /// Origin of an object at `pointer`, when the source path is known.
    pub fn origin(&self, pointer: &str) -> Option<Origin> {
        self.source_path
            .as_ref()
            .map(|path| Origin::file(path.clone()).with_pointer(pointer))
    }

    pub fn report(&mut self, severity: Severity, code: &str, message: String, pointer: &str) {
        let origin =
            Origin::file(self.source_path.clone().unwrap_or_default()).with_pointer(pointer);
        self.diagnostics
            .push(Diagnostic::new(severity, code, message).with_origin(origin));
    }

    fn observe_key_order(&mut self, map: &Map<String, Value>) {
        if self.alphabetical {
            let mut keys = map.keys();
            if let Some(mut prev) = keys.next() {
                for key in keys {
                    if alphabetical_cmp(prev, key) == Ordering::Greater {
                        self.alphabetical = false;
                        return;
                    }
                    prev = key;
                }
            }
        }
    }
}

/// `name` first, then ordinal order.
pub(crate) fn alphabetical_cmp(a: &str, b: &str) -> Ordering {
    (a != "name", a.as_bytes()).cmp(&(b != "name", b.as_bytes()))
}

pub(crate) fn absent_marker() -> PropValue {
    PropValue::List(Vec::new())
}

pub(crate) fn is_absent_marker(value: &PropValue) -> bool {
    matches!(value, PropValue::List(items) if items.is_empty())
}

/// JSON pointer of `key` inside the object at `base` (RFC 6901 escaping).
pub(crate) fn child_pointer(base: &str, key: &str) -> String {
    let mut out = String::with_capacity(base.len() + key.len() + 1);
    out.push_str(base);
    out.push('/');
    for c in key.chars() {
        match c {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            c => out.push(c),
        }
    }
    out
}

/// Reads one JSON object: typed properties are taken with codecs; the rest
/// (plus shadows) ends up in the `extra` bag returned by [`Self::finish`].
pub(crate) struct ObjReader<'v> {
    map: &'v Map<String, Value>,
    pointer: String,
    claimed: Vec<&'static str>,
    shadows: Vec<(&'static str, PropValue)>,
}

impl<'v> ObjReader<'v> {
    pub fn new(value: &'v Value, pointer: String, cx: &mut ReadCx) -> Result<Self, TmslError> {
        let Value::Object(map) = value else {
            return Err(TmslError::unexpected(&pointer, "an object", value));
        };
        cx.observe_key_order(map);
        Ok(Self {
            map,
            pointer,
            claimed: Vec::new(),
            shadows: Vec::new(),
        })
    }

    pub fn pointer(&self) -> &str {
        &self.pointer
    }

    pub fn child(&self, key: &str) -> String {
        child_pointer(&self.pointer, key)
    }

    /// Decodes a typed property; wrong JSON types are errors.
    pub fn take<C: Codec>(&mut self, key: &'static str, codec: &C) -> Result<C::T, TmslError> {
        self.claimed.push(key);
        let original = self.map.get(key);
        let value = codec.decode(original, &self.child(key))?;
        self.shadow_if_needed(key, original, codec.encode(&value));
        Ok(value)
    }

    /// Like [`Self::take`], but a value the codec cannot represent is kept
    /// verbatim in `extra` (the typed field gets its default) and reported.
    pub fn take_lenient<C: Codec>(
        &mut self,
        key: &'static str,
        codec: &C,
        cx: &mut ReadCx,
        (severity, code, what): (Severity, &str, &str),
    ) -> Result<C::T, TmslError> {
        self.claimed.push(key);
        let pointer = self.child(key);
        let original = self.map.get(key);
        match codec.decode(original, &pointer) {
            Ok(value) => {
                self.shadow_if_needed(key, original, codec.encode(&value));
                Ok(value)
            }
            Err(err) => {
                if let Some(original) = original {
                    self.shadows.push((key, PropValue::Json(original.clone())));
                }
                cx.report(
                    severity,
                    code,
                    format!("{what} kept verbatim: {err}"),
                    &pointer,
                );
                codec.decode(None, &pointer)
            }
        }
    }

    /// A child collection (`tables`, `columns`, …). Absent is empty; an
    /// explicit empty array is kept as a shadow.
    pub fn array(&mut self, key: &'static str) -> Result<&'v [Value], TmslError> {
        self.claimed.push(key);
        match self.map.get(key) {
            None => Ok(&[]),
            Some(Value::Array(items)) => {
                if items.is_empty() {
                    self.shadows
                        .push((key, PropValue::Json(Value::Array(Vec::new()))));
                }
                Ok(items)
            }
            Some(other) => Err(TmslError::unexpected(&self.child(key), "an array", other)),
        }
    }

    /// Claims a key whose value the caller interprets itself.
    pub fn raw(&mut self, key: &'static str) -> Option<&'v Value> {
        self.claimed.push(key);
        self.map.get(key)
    }

    /// Stores `value` under a claimed key in the `extra` bag.
    pub fn keep(&mut self, key: &'static str, value: PropValue) {
        self.shadows.push((key, value));
    }

    fn shadow_if_needed(
        &mut self,
        key: &'static str,
        original: Option<&Value>,
        encoded: Option<Value>,
    ) {
        if encoded.as_ref() != original {
            let shadow = original.map_or_else(absent_marker, |v| PropValue::Json(v.clone()));
            self.shadows.push((key, shadow));
        }
    }

    /// Unclaimed keys and shadows, in source order (absent markers last).
    pub fn finish(self) -> PropertyBag {
        let mut bag = PropertyBag::new();
        for (key, value) in self.map {
            if self.claimed.contains(&key.as_str()) {
                if let Some((_, shadow)) = self.shadows.iter().find(|(k, _)| k == key) {
                    bag.insert(key.clone(), shadow.clone());
                }
            } else {
                bag.insert(key.clone(), PropValue::Json(value.clone()));
            }
        }
        for (key, shadow) in self.shadows {
            if !self.map.contains_key(key) {
                bag.insert(key.to_owned(), shadow);
            }
        }
        bag
    }
}

/// Builds one JSON object from typed fields plus an `extra` bag.
pub(crate) struct ObjWriter<'a> {
    extra: &'a PropertyBag,
    claimed: Vec<&'static str>,
    /// Claimed keys written from their `extra` entry (shadows, verbatim
    /// values); reading the output back puts them in `extra` again.
    from_extra: Vec<&'static str>,
    out: Vec<(String, Value)>,
}

impl<'a> ObjWriter<'a> {
    pub fn new(extra: &'a PropertyBag) -> Self {
        Self {
            extra,
            claimed: Vec::new(),
            from_extra: Vec::new(),
            out: Vec::new(),
        }
    }

    /// Writes a typed property, preferring a matching shadow from `extra`.
    pub fn put<C: Codec>(&mut self, key: &'static str, codec: &C, value: &C::T) {
        self.claimed.push(key);
        let encoded = codec.encode(value);
        if let Some(shadow) = self.extra.get(key) {
            if is_absent_marker(shadow) {
                let default = codec.decode(None, "").ok();
                if default.is_some_and(|d| codec.encode(&d) == encoded) {
                    return;
                }
            } else {
                let raw = shadow.to_json();
                match codec.decode(Some(&raw), "") {
                    Ok(decoded) if codec.encode(&decoded) == encoded => {
                        self.out.push((key.to_owned(), raw));
                        self.from_extra.push(key);
                        return;
                    }
                    // Kept verbatim because the model could not represent it.
                    Err(_) if encoded.is_none() => {
                        self.out.push((key.to_owned(), raw));
                        self.from_extra.push(key);
                        return;
                    }
                    _ => {}
                }
            }
        }
        if let Some(value) = encoded {
            self.out.push((key.to_owned(), value));
        }
    }

    /// Writes a child collection; empty collections are omitted unless the
    /// source had an explicit `[]`.
    pub fn put_collection(&mut self, key: &'static str, items: Vec<Value>) {
        self.claimed.push(key);
        if !items.is_empty() {
            self.out.push((key.to_owned(), Value::Array(items)));
        } else if self
            .extra
            .get(key)
            .is_some_and(|v| matches!(v, PropValue::Json(Value::Array(a)) if a.is_empty()))
        {
            self.out.push((key.to_owned(), Value::Array(Vec::new())));
            self.from_extra.push(key);
        }
    }

    /// Writes a value built by the caller.
    pub fn put_value(&mut self, key: &'static str, value: Value) {
        self.claimed.push(key);
        self.out.push((key.to_owned(), value));
    }

    /// Marks a key as handled without writing it.
    pub fn claim(&mut self, key: &'static str) {
        self.claimed.push(key);
    }

    /// Merges typed entries with unclaimed `extra` entries and orders them.
    ///
    /// [`KeyOrder::Tom`]: keys listed in `order` take their canonical slot;
    /// an unknown key is placed right after the nearest preceding `extra`
    /// key that is listed in `order` (its anchor), keeping source order
    /// among themselves. An unknown key without an anchor goes where reading
    /// the output back would anchor it — right after the last written key
    /// before the `*` slot that comes back into `extra` — or else at the `*`
    /// slot, so writing what was read is a fixed point.
    pub fn finish(self, order: &[&str], key_order: KeyOrder) -> Value {
        let position = |key: &str| order.iter().position(|k| *k == key);
        let star = position("*").map_or(usize::MAX, |i| i * 2);
        let unanchored = self
            .extra
            .iter()
            .filter(|(key, value)| {
                if self.claimed.contains(&key.as_str()) {
                    self.from_extra.contains(&key.as_str())
                } else {
                    !is_absent_marker(value)
                }
            })
            .filter_map(|(key, _)| position(key).filter(|i| i * 2 < star))
            .max()
            .map_or(star, |i| i * 2 + 1);
        let mut entries: Vec<((usize, usize), String, Value)> = self
            .out
            .into_iter()
            .map(|(key, value)| {
                let rank = position(&key).map_or(star, |i| i * 2);
                ((rank, 0), key, value)
            })
            .collect();
        let mut anchor: Option<usize> = None;
        for (seq, (key, value)) in self.extra.iter().enumerate() {
            let slot = position(key);
            if let Some(i) = slot {
                anchor = Some(i);
            }
            if self.claimed.contains(&key.as_str()) || is_absent_marker(value) {
                continue;
            }
            let rank = match (slot, anchor) {
                (Some(i), _) => i * 2,
                (None, Some(a)) => a * 2 + 1,
                (None, None) => unanchored,
            };
            entries.push(((rank, seq + 1), key.clone(), value.to_json()));
        }
        match key_order {
            KeyOrder::Tom => entries.sort_by_key(|(rank, _, _)| *rank),
            KeyOrder::Alphabetical => entries.sort_by(|a, b| alphabetical_cmp(&a.1, &b.1)),
        }
        Value::Object(entries.into_iter().map(|(_, k, v)| (k, v)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::{FLAG, OPT_SINGLE};
    use serde_json::json;

    #[test]
    fn unknown_keys_follow_their_anchor_or_the_star_slot() {
        let order = ["name", "a", "b", "*", "annotations"];
        let mut extra = PropertyBag::new();
        extra.insert("lead".into(), json!(1).into());
        extra.insert("a".into(), json!(2).into());
        extra.insert("afterA".into(), json!(3).into());
        let mut w = ObjWriter::new(&extra);
        w.put("name", &OPT_SINGLE, &Some("n".to_owned()));
        w.put("b", &FLAG, &true);
        w.put_value("annotations", json!([]));
        let keys: Vec<String> = match w.finish(&order, KeyOrder::Tom) {
            Value::Object(map) => map.keys().cloned().collect(),
            _ => Vec::new(),
        };
        // `lead` has no anchor; it goes after `a`, where a re-read anchors it.
        assert_eq!(keys, ["name", "a", "lead", "afterA", "b", "annotations"]);
    }

    #[test]
    fn unanchored_keys_use_the_star_slot_without_extra_keys_before_it() {
        let order = ["name", "a", "*", "annotations"];
        let mut extra = PropertyBag::new();
        extra.insert("lead".into(), json!(1).into());
        extra.insert("name".into(), absent_marker());
        let mut w = ObjWriter::new(&extra);
        w.put("a", &FLAG, &true);
        w.put_value("annotations", json!([]));
        let keys: Vec<String> = match w.finish(&order, KeyOrder::Tom) {
            Value::Object(map) => map.keys().cloned().collect(),
            _ => Vec::new(),
        };
        assert_eq!(keys, ["a", "lead", "annotations"]);
    }

    #[test]
    fn stale_shadows_lose_to_typed_values() {
        let mut extra = PropertyBag::new();
        extra.insert("isHidden".into(), json!(false).into());
        let write = |value: bool| {
            let mut w = ObjWriter::new(&extra);
            w.put("isHidden", &FLAG, &value);
            w.finish(&["isHidden"], KeyOrder::Tom)
        };
        assert_eq!(write(false), json!({"isHidden": false}));
        assert_eq!(write(true), json!({"isHidden": true}));
    }

    #[test]
    fn pointers_are_escaped() {
        assert_eq!(child_pointer("/model", "a/b~c"), "/model/a~1b~0c");
    }
}
