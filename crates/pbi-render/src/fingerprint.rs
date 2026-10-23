//! Content fingerprints for render caches ([`crate::visuals::cache`]).
//!
//! [`Fingerprint::add`] feeds any `Serialize` value into a hasher through a
//! [`serde::Serializer`] that writes type tags, lengths and raw bits (no
//! text is built, floats hash by their bits). [`Fingerprint::add_dataview`]
//! hashes a [`DataView`] directly: its cells are the bulk of a page's data
//! and their `Serialize` impl converts every value to JSON first.
//!
//! Fingerprints only live in memory (never persisted), so the hasher's
//! algorithm may change between builds. Equal inputs always give equal
//! fingerprints; maps hash in iteration order (`IndexMap`, `serde_json`
//! with `preserve_order`), so a container with unordered iteration would at
//! worst cause cache misses, never false hits.

use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};

use pbi_engine::Value;
use serde::Serialize;
use serde::ser::{self, Serializer};

use crate::dataview::{
    Categorical, CategoryColumn, CellValue, DataView, HiddenValues, Matrix, MatrixColumn,
    MatrixNode, SlicerData, TableData, ValueColumn,
};

/// A 64-bit content hash under construction. Inputs are buffered and
/// hashed in one pass by [`Fingerprint::finish`] (many tiny `Hasher`
/// writes are much slower than one long one).
#[derive(Debug, Clone, Default)]
pub(crate) struct Fingerprint {
    hasher: Buffer,
    /// A `Serialize` impl reported an error: the fingerprint is unusable.
    failed: bool,
}

/// A [`Hasher`] that only collects the bytes written to it.
#[derive(Debug, Clone, Default)]
struct Buffer(Vec<u8>);

impl Hasher for Buffer {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    fn write_u8(&mut self, v: u8) {
        self.0.push(v);
    }

    fn finish(&self) -> u64 {
        let mut h = DefaultHasher::new();
        h.write(&self.0);
        h.finish()
    }
}

impl Fingerprint {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Adds a raw number (a nested fingerprint, a tag).
    pub(crate) fn add_u64(&mut self, v: u64) -> &mut Self {
        self.hasher.write_u64(v);
        self
    }

    /// Adds any serializable value.
    pub(crate) fn add<T: Serialize + ?Sized>(&mut self, value: &T) -> &mut Self {
        if value.serialize(HashSer(&mut self.hasher)).is_err() {
            self.failed = true;
        }
        self
    }

    /// Adds a DataView (every field, cells hashed directly).
    pub(crate) fn add_dataview(&mut self, dv: &DataView) -> &mut Self {
        let DataView {
            kind,
            metadata,
            single,
            categorical,
            table,
            matrix,
            slicer,
            truncated,
            warnings,
        } = dv;
        self.add(kind).add(metadata);
        let h = &mut self.hasher;
        opt_cell(h, single.as_ref());
        match categorical {
            Some(c) => {
                1u8.hash(h);
                hash_categorical(h, c);
            }
            None => 0u8.hash(h),
        }
        match table {
            Some(t) => {
                1u8.hash(h);
                hash_table(h, t);
            }
            None => 0u8.hash(h),
        }
        match matrix {
            Some(m) => {
                1u8.hash(h);
                hash_matrix(h, m);
            }
            None => 0u8.hash(h),
        }
        match slicer {
            Some(s) => {
                1u8.hash(h);
                let SlicerData {
                    selected,
                    inverted,
                    all_selected,
                    values,
                } = s;
                selected.hash(h);
                (inverted, all_selected).hash(h);
                values.len().hash(h);
                for tuple in values {
                    cells(h, tuple);
                }
            }
            None => 0u8.hash(h),
        }
        truncated.hash(h);
        self.add(warnings)
    }

    /// The fingerprint, or `None` if a value could not be hashed.
    pub(crate) fn finish(&self) -> Option<u64> {
        (!self.failed).then(|| self.hasher.finish())
    }
}

fn value(h: &mut Buffer, v: &Value) {
    match v {
        Value::Blank => 0u8.hash(h),
        Value::Boolean(b) => (1u8, b).hash(h),
        Value::Integer(i) => (2u8, i).hash(h),
        Value::Double(f) => (3u8, f.to_bits()).hash(h),
        Value::Decimal(d) => (4u8, d).hash(h),
        Value::String(s) => {
            5u8.hash(h);
            s.hash(h);
        }
        Value::DateTime(t) => (6u8, t).hash(h),
    }
}

fn cell(h: &mut Buffer, c: &CellValue) {
    let CellValue {
        raw,
        formatted,
        format,
    } = c;
    value(h, raw);
    formatted.hash(h);
    // One byte for the common case (no own format string).
    match format {
        None => h.write_u8(0),
        Some(f) => {
            h.write_u8(1);
            f.hash(h);
        }
    }
}

fn cells(h: &mut Buffer, cs: &[CellValue]) {
    cs.len().hash(h);
    for c in cs {
        cell(h, c);
    }
}

fn opt_cell(h: &mut Buffer, c: Option<&CellValue>) {
    match c {
        Some(c) => {
            1u8.hash(h);
            cell(h, c);
        }
        None => 0u8.hash(h),
    }
}

fn opt_cells(h: &mut Buffer, cs: Option<&Vec<CellValue>>) {
    match cs {
        Some(cs) => {
            1u8.hash(h);
            cells(h, cs);
        }
        None => 0u8.hash(h),
    }
}

fn hash_hidden(h: &mut Buffer, hidden: &[HiddenValues]) {
    hidden.len().hash(h);
    for HiddenValues {
        meta_idx,
        values,
        total,
    } in hidden
    {
        meta_idx.hash(h);
        cells(h, values);
        opt_cell(h, total.as_ref());
    }
}

fn hash_categorical(h: &mut Buffer, c: &Categorical) {
    let Categorical {
        categories,
        values,
        series_meta_idx,
        hidden,
    } = c;
    categories.len().hash(h);
    for CategoryColumn { meta_idx, values } in categories {
        meta_idx.hash(h);
        cells(h, values);
    }
    values.len().hash(h);
    for ValueColumn {
        meta_idx,
        series,
        values,
        highlights,
    } in values
    {
        meta_idx.hash(h);
        opt_cell(h, series.as_ref());
        cells(h, values);
        opt_cells(h, highlights.as_ref());
    }
    series_meta_idx.hash(h);
    hash_hidden(h, hidden);
}

fn hash_table(h: &mut Buffer, t: &TableData) {
    let TableData {
        columns,
        rows,
        totals,
        hidden,
    } = t;
    columns.hash(h);
    rows.len().hash(h);
    for row in rows {
        cells(h, row);
    }
    opt_cells(h, totals.as_ref());
    hash_hidden(h, hidden);
}

fn hash_matrix(h: &mut Buffer, m: &Matrix) {
    let Matrix {
        row_levels,
        column_levels,
        values,
        columns,
        rows,
        row_subtotals,
        column_subtotals,
        hidden,
    } = m;
    (row_levels, column_levels, values, hidden).hash(h);
    columns.len().hash(h);
    for MatrixColumn { path, is_total } in columns {
        cells(h, path);
        is_total.hash(h);
    }
    // Iterative walk: untrusted depth must not overflow the stack.
    let mut stack: Vec<&MatrixNode> = vec![rows];
    while let Some(node) = stack.pop() {
        let MatrixNode {
            level,
            value,
            cells: node_cells,
            children,
            hidden: node_hidden,
        } = node;
        level.hash(h);
        opt_cell(h, value.as_ref());
        cells(h, node_cells);
        cells(h, node_hidden);
        children.len().hash(h);
        stack.extend(children.iter().rev());
    }
    (row_subtotals, column_subtotals).hash(h);
}

/// Never produced by [`HashSer`] itself; carries a `Serialize` impl's
/// custom error.
#[derive(Debug)]
struct Unhashable;

impl fmt::Display for Unhashable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("value cannot be fingerprinted")
    }
}

impl std::error::Error for Unhashable {}

impl ser::Error for Unhashable {
    fn custom<T: fmt::Display>(_msg: T) -> Self {
        Unhashable
    }
}

/// Type tags of the serde data model (keep values distinct).
mod tag {
    pub const BOOL: u8 = 1;
    pub const INT: u8 = 2;
    pub const UINT: u8 = 3;
    pub const FLOAT: u8 = 4;
    pub const CHAR: u8 = 5;
    pub const STR: u8 = 6;
    pub const BYTES: u8 = 7;
    pub const NONE: u8 = 8;
    pub const SOME: u8 = 9;
    pub const UNIT: u8 = 10;
    pub const UNIT_STRUCT: u8 = 11;
    pub const UNIT_VARIANT: u8 = 12;
    pub const NEWTYPE_STRUCT: u8 = 13;
    pub const NEWTYPE_VARIANT: u8 = 14;
    pub const SEQ: u8 = 15;
    pub const TUPLE: u8 = 16;
    pub const TUPLE_STRUCT: u8 = 17;
    pub const TUPLE_VARIANT: u8 = 18;
    pub const MAP: u8 = 19;
    pub const STRUCT: u8 = 20;
    pub const STRUCT_VARIANT: u8 = 21;
    pub const ELEMENT: u8 = 22;
    pub const KEY: u8 = 23;
    pub const VALUE: u8 = 24;
    pub const FIELD: u8 = 25;
    pub const SKIPPED: u8 = 26;
    pub const END: u8 = 27;
}

/// A serializer that hashes the serde data model of a value.
struct HashSer<'a>(&'a mut Buffer);

impl HashSer<'_> {
    fn tag(&mut self, t: u8) {
        self.0.write_u8(t);
    }

    fn nested<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Unhashable> {
        value.serialize(HashSer(&mut *self.0))
    }
}

impl Serializer for HashSer<'_> {
    type Ok = ();
    type Error = Unhashable;
    type SerializeSeq = Self;
    type SerializeTuple = Self;
    type SerializeTupleStruct = Self;
    type SerializeTupleVariant = Self;
    type SerializeMap = Self;
    type SerializeStruct = Self;
    type SerializeStructVariant = Self;

    fn serialize_bool(mut self, v: bool) -> Result<(), Unhashable> {
        self.tag(tag::BOOL);
        v.hash(self.0);
        Ok(())
    }

    fn serialize_i8(self, v: i8) -> Result<(), Unhashable> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i16(self, v: i16) -> Result<(), Unhashable> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i32(self, v: i32) -> Result<(), Unhashable> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i64(self, v: i64) -> Result<(), Unhashable> {
        self.serialize_i128(i128::from(v))
    }

    fn serialize_i128(mut self, v: i128) -> Result<(), Unhashable> {
        self.tag(tag::INT);
        v.hash(self.0);
        Ok(())
    }

    fn serialize_u8(self, v: u8) -> Result<(), Unhashable> {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u16(self, v: u16) -> Result<(), Unhashable> {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u32(self, v: u32) -> Result<(), Unhashable> {
        self.serialize_u64(u64::from(v))
    }

    fn serialize_u64(self, v: u64) -> Result<(), Unhashable> {
        self.serialize_u128(u128::from(v))
    }

    fn serialize_u128(mut self, v: u128) -> Result<(), Unhashable> {
        self.tag(tag::UINT);
        v.hash(self.0);
        Ok(())
    }

    fn serialize_f32(self, v: f32) -> Result<(), Unhashable> {
        self.serialize_f64(f64::from(v))
    }

    fn serialize_f64(mut self, v: f64) -> Result<(), Unhashable> {
        self.tag(tag::FLOAT);
        v.to_bits().hash(self.0);
        Ok(())
    }

    fn serialize_char(mut self, v: char) -> Result<(), Unhashable> {
        self.tag(tag::CHAR);
        u32::from(v).hash(self.0);
        Ok(())
    }

    fn serialize_str(mut self, v: &str) -> Result<(), Unhashable> {
        self.tag(tag::STR);
        v.hash(self.0);
        Ok(())
    }

    fn serialize_bytes(mut self, v: &[u8]) -> Result<(), Unhashable> {
        self.tag(tag::BYTES);
        v.hash(self.0);
        Ok(())
    }

    fn serialize_none(mut self) -> Result<(), Unhashable> {
        self.tag(tag::NONE);
        Ok(())
    }

    fn serialize_some<T: Serialize + ?Sized>(mut self, value: &T) -> Result<(), Unhashable> {
        self.tag(tag::SOME);
        self.nested(value)
    }

    fn serialize_unit(mut self) -> Result<(), Unhashable> {
        self.tag(tag::UNIT);
        Ok(())
    }

    fn serialize_unit_struct(mut self, name: &'static str) -> Result<(), Unhashable> {
        self.tag(tag::UNIT_STRUCT);
        name.hash(self.0);
        Ok(())
    }

    fn serialize_unit_variant(
        mut self,
        _name: &'static str,
        index: u32,
        variant: &'static str,
    ) -> Result<(), Unhashable> {
        self.tag(tag::UNIT_VARIANT);
        (index, variant).hash(self.0);
        Ok(())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        mut self,
        _name: &'static str,
        value: &T,
    ) -> Result<(), Unhashable> {
        self.tag(tag::NEWTYPE_STRUCT);
        self.nested(value)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        mut self,
        _name: &'static str,
        index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Unhashable> {
        self.tag(tag::NEWTYPE_VARIANT);
        (index, variant).hash(self.0);
        self.nested(value)
    }

    fn serialize_seq(mut self, _len: Option<usize>) -> Result<Self, Unhashable> {
        self.tag(tag::SEQ);
        Ok(self)
    }

    fn serialize_tuple(mut self, len: usize) -> Result<Self, Unhashable> {
        self.tag(tag::TUPLE);
        len.hash(self.0);
        Ok(self)
    }

    fn serialize_tuple_struct(
        mut self,
        _name: &'static str,
        len: usize,
    ) -> Result<Self, Unhashable> {
        self.tag(tag::TUPLE_STRUCT);
        len.hash(self.0);
        Ok(self)
    }

    fn serialize_tuple_variant(
        mut self,
        _name: &'static str,
        index: u32,
        variant: &'static str,
        len: usize,
    ) -> Result<Self, Unhashable> {
        self.tag(tag::TUPLE_VARIANT);
        (index, variant, len).hash(self.0);
        Ok(self)
    }

    fn serialize_map(mut self, _len: Option<usize>) -> Result<Self, Unhashable> {
        self.tag(tag::MAP);
        Ok(self)
    }

    fn serialize_struct(mut self, name: &'static str, _len: usize) -> Result<Self, Unhashable> {
        self.tag(tag::STRUCT);
        // `serde_json`'s arbitrary-precision numbers are a struct with this
        // name and one string field; the name keeps them apart from text.
        name.hash(self.0);
        Ok(self)
    }

    fn serialize_struct_variant(
        mut self,
        _name: &'static str,
        index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<Self, Unhashable> {
        self.tag(tag::STRUCT_VARIANT);
        (index, variant).hash(self.0);
        Ok(self)
    }
}

impl ser::SerializeSeq for HashSer<'_> {
    type Ok = ();
    type Error = Unhashable;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Unhashable> {
        self.tag(tag::ELEMENT);
        self.nested(value)
    }

    fn end(mut self) -> Result<(), Unhashable> {
        self.tag(tag::END);
        Ok(())
    }
}

impl ser::SerializeTuple for HashSer<'_> {
    type Ok = ();
    type Error = Unhashable;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Unhashable> {
        self.tag(tag::ELEMENT);
        self.nested(value)
    }

    fn end(mut self) -> Result<(), Unhashable> {
        self.tag(tag::END);
        Ok(())
    }
}

impl ser::SerializeTupleStruct for HashSer<'_> {
    type Ok = ();
    type Error = Unhashable;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Unhashable> {
        self.tag(tag::ELEMENT);
        self.nested(value)
    }

    fn end(mut self) -> Result<(), Unhashable> {
        self.tag(tag::END);
        Ok(())
    }
}

impl ser::SerializeTupleVariant for HashSer<'_> {
    type Ok = ();
    type Error = Unhashable;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Unhashable> {
        self.tag(tag::ELEMENT);
        self.nested(value)
    }

    fn end(mut self) -> Result<(), Unhashable> {
        self.tag(tag::END);
        Ok(())
    }
}

impl ser::SerializeMap for HashSer<'_> {
    type Ok = ();
    type Error = Unhashable;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Unhashable> {
        self.tag(tag::KEY);
        self.nested(key)
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Unhashable> {
        self.tag(tag::VALUE);
        self.nested(value)
    }

    fn end(mut self) -> Result<(), Unhashable> {
        self.tag(tag::END);
        Ok(())
    }
}

impl ser::SerializeStruct for HashSer<'_> {
    type Ok = ();
    type Error = Unhashable;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Unhashable> {
        self.tag(tag::FIELD);
        key.hash(self.0);
        self.nested(value)
    }

    fn skip_field(&mut self, key: &'static str) -> Result<(), Unhashable> {
        self.tag(tag::SKIPPED);
        key.hash(self.0);
        Ok(())
    }

    fn end(mut self) -> Result<(), Unhashable> {
        self.tag(tag::END);
        Ok(())
    }
}

impl ser::SerializeStructVariant for HashSer<'_> {
    type Ok = ();
    type Error = Unhashable;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Unhashable> {
        self.tag(tag::FIELD);
        key.hash(self.0);
        self.nested(value)
    }

    fn skip_field(&mut self, key: &'static str) -> Result<(), Unhashable> {
        self.tag(tag::SKIPPED);
        key.hash(self.0);
        Ok(())
    }

    fn end(mut self) -> Result<(), Unhashable> {
        self.tag(tag::END);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fp<T: Serialize + ?Sized>(v: &T) -> u64 {
        Fingerprint::new().add(v).finish().unwrap_or(0)
    }

    #[test]
    fn distinguishes_structure_and_values() {
        assert_eq!(fp(&("a", 1.5)), fp(&("a", 1.5)));
        assert_ne!(fp(&("a", 1.5)), fp(&("a", 1.25)));
        assert_ne!(fp(&vec!["ab", "c"]), fp(&vec!["a", "bc"]));
        assert_ne!(fp(&vec![vec![1], vec![]]), fp(&vec![vec![], vec![1]]));
        assert_ne!(fp(&Some(0u8)), fp(&None::<u8>));
        let json = |s: &str| serde_json::from_str::<serde_json::Value>(s).unwrap_or_default();
        assert_eq!(fp(&json(r#"{"a":[1,2.5]}"#)), fp(&json(r#"{"a":[1,2.5]}"#)));
        assert_ne!(fp(&json(r#"{"a":1}"#)), fp(&json(r#"{"a":"1"}"#)));
        assert_ne!(fp(&json(r#"{"a":1}"#)), fp(&json(r#"{"b":1}"#)));
    }

    #[test]
    fn dataview_cells_change_the_fingerprint() {
        let table = |v: i64, text: &str| DataView {
            table: Some(TableData {
                columns: vec![0],
                rows: vec![vec![CellValue::new(Value::Integer(v), text)]],
                totals: None,
                hidden: Vec::new(),
            }),
            ..DataView::default()
        };
        let of = |dv: &DataView| Fingerprint::new().add_dataview(dv).finish();
        assert_eq!(of(&table(1, "1")), of(&table(1, "1")));
        assert_ne!(of(&table(1, "1")), of(&table(2, "1")));
        assert_ne!(of(&table(1, "1")), of(&table(1, "01")));
        assert_ne!(of(&table(1, "1")), of(&DataView::default()));
        // A cell's own format string and the hidden values count too.
        let mut formatted = table(1, "1");
        if let Some(t) = formatted.table.as_mut() {
            t.rows[0][0] = CellValue::new(Value::Integer(1), "1").with_format("0");
        }
        assert_ne!(of(&table(1, "1")), of(&formatted));
        let mut hidden = table(1, "1");
        if let Some(t) = hidden.table.as_mut() {
            t.hidden.push(HiddenValues {
                meta_idx: 1,
                values: vec![CellValue::new(Value::from("Red"), "Red")],
                total: None,
            });
        }
        assert_ne!(of(&table(1, "1")), of(&hidden));
    }
}
