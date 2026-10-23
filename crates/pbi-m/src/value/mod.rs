//! The M value model.
//!
//! * [`Value`] is an evaluated M value. Compound values ([`ListValue`], [`RecordValue`],
//!   [`TableValue`]) hold [`Lazy`] slots: list items and record fields are evaluated on demand
//!   (M is lazy), and a slot may hold an error that only surfaces when the slot is accessed.
//! * Values are immutable and cheap to clone (`Arc`-shared).
//! * Metadata (`x meta [...]`) is carried by [`Value::Meta`]; every consumer looks through it
//!   with [`Value::plain`], so it never changes how a value behaves.
//!
//! Values returned by [`crate::Engine`] are *resolved*: every reachable slot outside function
//! bodies has been evaluated, so [`Lazy::resolved`] never needs the evaluator.

pub(crate) mod drop;
pub(crate) mod format;
pub(crate) mod ops;
pub mod table;
pub mod types;

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeDelta};

use crate::ast::PrimitiveType;
use crate::eval::env::Env;
use crate::eval::interp::Interp;
use crate::eval::ir::FnDef;
use crate::merror::{LimitKind, MError};

pub use crate::eval::thunk::Thunk;
pub use table::TableValue;
pub use types::{
    Facet, FieldType, FunctionType, ParamType, RecordType, TableType, TypeKind, TypeValue,
};

/// An M value.
#[derive(Clone)]
pub enum Value {
    Null,
    Logical(bool),
    /// M numbers are IEEE doubles. Integer/decimal facets (`Int64.Type`, `Currency.Type`) live on
    /// types (see [`Facet`]), not on values.
    Number(f64),
    Text(Arc<str>),
    Date(NaiveDate),
    Time(NaiveTime),
    DateTime(NaiveDateTime),
    DateTimeZone(DateTime<FixedOffset>),
    Duration(TimeDelta),
    Binary(BinaryValue),
    List(ListValue),
    Record(RecordValue),
    Table(TableValue),
    Function(FunctionValue),
    Type(TypeValue),
    /// A value with a metadata record (`value meta [...]`).
    Meta(Arc<MetaValue>),
}

/// A value together with its metadata record.
#[derive(Debug, Clone)]
pub struct MetaValue {
    pub value: Value,
    pub meta: RecordValue,
}

impl Drop for MetaValue {
    fn drop(&mut self) {
        let v = std::mem::replace(&mut self.value, Value::Null);
        if !matches!(v, Value::Null) {
            drop::defer(drop::Garbage::Value(v));
        }
    }
}

/// The kind of a (plain) value, i.e. its primitive type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValueKind {
    Null,
    Logical,
    Number,
    Text,
    Date,
    Time,
    DateTime,
    DateTimeZone,
    Duration,
    Binary,
    List,
    Record,
    Table,
    Function,
    Type,
}

impl ValueKind {
    /// Capitalized type name as used in M error messages (`Text`, `Number`, ...).
    pub const fn name(self) -> &'static str {
        match self {
            ValueKind::Null => "Null",
            ValueKind::Logical => "Logical",
            ValueKind::Number => "Number",
            ValueKind::Text => "Text",
            ValueKind::Date => "Date",
            ValueKind::Time => "Time",
            ValueKind::DateTime => "DateTime",
            ValueKind::DateTimeZone => "DateTimeZone",
            ValueKind::Duration => "Duration",
            ValueKind::Binary => "Binary",
            ValueKind::List => "List",
            ValueKind::Record => "Record",
            ValueKind::Table => "Table",
            ValueKind::Function => "Function",
            ValueKind::Type => "Type",
        }
    }

    pub const fn primitive(self) -> PrimitiveType {
        match self {
            ValueKind::Null => PrimitiveType::Null,
            ValueKind::Logical => PrimitiveType::Logical,
            ValueKind::Number => PrimitiveType::Number,
            ValueKind::Text => PrimitiveType::Text,
            ValueKind::Date => PrimitiveType::Date,
            ValueKind::Time => PrimitiveType::Time,
            ValueKind::DateTime => PrimitiveType::DateTime,
            ValueKind::DateTimeZone => PrimitiveType::DateTimeZone,
            ValueKind::Duration => PrimitiveType::Duration,
            ValueKind::Binary => PrimitiveType::Binary,
            ValueKind::List => PrimitiveType::List,
            ValueKind::Record => PrimitiveType::Record,
            ValueKind::Table => PrimitiveType::Table,
            ValueKind::Function => PrimitiveType::Function,
            ValueKind::Type => PrimitiveType::Type,
        }
    }
}

impl Value {
    pub fn text(s: impl Into<Arc<str>>) -> Value {
        Value::Text(s.into())
    }

    pub fn number(n: f64) -> Value {
        Value::Number(n)
    }

    pub fn list(items: impl IntoIterator<Item = Value>) -> Value {
        Value::List(ListValue::from_values(items))
    }

    /// The value without metadata.
    pub fn plain(&self) -> &Value {
        let mut v = self;
        while let Value::Meta(m) = v {
            v = &m.value;
        }
        v
    }

    pub fn into_plain(self) -> Value {
        match self {
            Value::Meta(m) => m.value.plain().clone(),
            v => v,
        }
    }

    /// The metadata record (empty if none).
    pub fn metadata(&self) -> RecordValue {
        match self {
            Value::Meta(m) => m.meta.clone(),
            _ => RecordValue::empty(),
        }
    }

    /// `self meta meta`: merges `meta` into existing metadata (new fields win).
    pub fn with_meta(self, meta: RecordValue) -> Value {
        match self {
            Value::Meta(m) => {
                let merged = m.meta.merge(&meta);
                Value::Meta(Arc::new(MetaValue {
                    value: m.value.clone(),
                    meta: merged,
                }))
            }
            v => Value::Meta(Arc::new(MetaValue { value: v, meta })),
        }
    }

    pub fn kind(&self) -> ValueKind {
        match self.plain() {
            Value::Null | Value::Meta(_) => ValueKind::Null,
            Value::Logical(_) => ValueKind::Logical,
            Value::Number(_) => ValueKind::Number,
            Value::Text(_) => ValueKind::Text,
            Value::Date(_) => ValueKind::Date,
            Value::Time(_) => ValueKind::Time,
            Value::DateTime(_) => ValueKind::DateTime,
            Value::DateTimeZone(_) => ValueKind::DateTimeZone,
            Value::Duration(_) => ValueKind::Duration,
            Value::Binary(_) => ValueKind::Binary,
            Value::List(_) => ValueKind::List,
            Value::Record(_) => ValueKind::Record,
            Value::Table(_) => ValueKind::Table,
            Value::Function(_) => ValueKind::Function,
            Value::Type(_) => ValueKind::Type,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.plain(), Value::Null)
    }

    pub fn as_text(&self) -> Option<&str> {
        match self.plain() {
            Value::Text(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self.plain() {
            Value::Number(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_logical(&self) -> Option<bool> {
        match self.plain() {
            Value::Logical(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&ListValue> {
        match self.plain() {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    pub fn as_record(&self) -> Option<&RecordValue> {
        match self.plain() {
            Value::Record(r) => Some(r),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<&TableValue> {
        match self.plain() {
            Value::Table(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_function(&self) -> Option<&FunctionValue> {
        match self.plain() {
            Value::Function(f) => Some(f),
            _ => None,
        }
    }

    pub fn as_type(&self) -> Option<&TypeValue> {
        match self.plain() {
            Value::Type(t) => Some(t),
            _ => None,
        }
    }

    pub fn as_binary(&self) -> Option<&BinaryValue> {
        match self.plain() {
            Value::Binary(b) => Some(b),
            _ => None,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.into())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s.into())
    }
}

impl From<f64> for Value {
    fn from(n: f64) -> Self {
        Value::Number(n)
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Logical(b)
    }
}

impl PartialEq for Value {
    /// M equality (`=`) on resolved values; slots that are errors or unevaluated compare unequal.
    fn eq(&self, other: &Self) -> bool {
        ops::equals(&ops::Resolved, self, other, 0).unwrap_or(false)
    }
}

impl fmt::Debug for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format::render(self, 4, 20))
    }
}

impl fmt::Display for Value {
    /// Renders the value as (approximately) M source, abbreviating large or deep values.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&format::render(self, 3, 10))
    }
}

// ---------------------------------------------------------------------------------------------
// Lazy slots
// ---------------------------------------------------------------------------------------------

/// A list item, record field or table cell: a value, an error, or a not-yet-evaluated
/// expression.
#[derive(Clone)]
pub enum Lazy {
    Value(Value),
    Error(Arc<MError>),
    Thunk(Arc<Thunk>),
}

impl Lazy {
    pub fn from_result(r: Result<Value, MError>) -> Lazy {
        match r {
            Ok(v) => Lazy::Value(v),
            Err(e) => Lazy::Error(Arc::new(e)),
        }
    }

    /// The value or error of a resolved slot. Unevaluated slots (only reachable through
    /// function closures) yield an internal error.
    pub fn resolved(&self) -> Result<Value, MError> {
        match self {
            Lazy::Value(v) => Ok(v.clone()),
            Lazy::Error(e) => Err((**e).clone()),
            Lazy::Thunk(t) => t.peek().unwrap_or_else(|| {
                Err(MError::Internal(
                    "value was not evaluated before the evaluation ended".into(),
                ))
            }),
        }
    }

    /// The value, if the slot holds one directly.
    pub fn as_value(&self) -> Option<&Value> {
        match self {
            Lazy::Value(v) => Some(v),
            _ => None,
        }
    }

    pub(crate) fn force(&self, it: &Interp) -> Result<Value, MError> {
        match self {
            Lazy::Value(v) => Ok(v.clone()),
            Lazy::Error(e) => Err((**e).clone()),
            Lazy::Thunk(t) => t.force(it),
        }
    }
}

impl From<Value> for Lazy {
    fn from(v: Value) -> Self {
        Lazy::Value(v)
    }
}

impl fmt::Debug for Lazy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Lazy::Value(v) => fmt::Debug::fmt(v, f),
            Lazy::Error(e) => write!(f, "Error({e})"),
            Lazy::Thunk(t) => match t.peek() {
                Some(Ok(v)) => fmt::Debug::fmt(&v, f),
                Some(Err(e)) => write!(f, "Error({e})"),
                None => f.write_str("<unevaluated>"),
            },
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------------------------

/// An M list. Items are [`Lazy`] slots.
#[derive(Clone)]
pub struct ListValue(Arc<Vec<Lazy>>);

impl ListValue {
    pub fn new(items: Vec<Lazy>) -> Self {
        Self(Arc::new(items))
    }

    pub fn from_values(items: impl IntoIterator<Item = Value>) -> Self {
        Self(Arc::new(items.into_iter().map(Lazy::Value).collect()))
    }

    pub fn empty() -> Self {
        Self(Arc::new(Vec::new()))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn items(&self) -> &[Lazy] {
        &self.0
    }

    pub fn ptr_eq(&self, other: &ListValue) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Drop for ListValue {
    fn drop(&mut self) {
        if let Some(items) = Arc::get_mut(&mut self.0)
            && !items.is_empty()
        {
            drop::defer(drop::Garbage::Slots(std::mem::take(items)));
        }
    }
}

impl fmt::Debug for ListValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&Value::List(self.clone()), f)
    }
}

// ---------------------------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------------------------

/// The ordered, duplicate-free field names of a record (shared by all rows of a table).
#[derive(Debug)]
pub struct Keys {
    names: Vec<Arc<str>>,
    index: Option<HashMap<Arc<str>, usize>>,
}

const KEY_INDEX_THRESHOLD: usize = 12;

impl Keys {
    /// Builds keys; returns the first duplicate name as the error.
    pub fn new(names: Vec<Arc<str>>) -> Result<Self, Arc<str>> {
        if names.len() > KEY_INDEX_THRESHOLD {
            let mut index = HashMap::with_capacity(names.len());
            for (i, n) in names.iter().enumerate() {
                if index.insert(n.clone(), i).is_some() {
                    return Err(n.clone());
                }
            }
            Ok(Self {
                names,
                index: Some(index),
            })
        } else {
            for (i, n) in names.iter().enumerate() {
                if names[..i].contains(n) {
                    return Err(n.clone());
                }
            }
            Ok(Self { names, index: None })
        }
    }

    /// Like [`Keys::new`], mapping a duplicate to the M error.
    pub fn try_new(names: Vec<Arc<str>>) -> Result<Self, MError> {
        Self::new(names).map_err(|d| {
            MError::expression(format!("The field '{d}' already exists in the record."))
        })
    }

    pub fn empty() -> Self {
        Self {
            names: Vec::new(),
            index: None,
        }
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn names(&self) -> &[Arc<str>] {
        &self.names
    }

    pub fn position(&self, name: &str) -> Option<usize> {
        match &self.index {
            Some(index) => index.get(name).copied(),
            None => self.names.iter().position(|n| &**n == name),
        }
    }
}

/// An M record: ordered named fields with [`Lazy`] values.
#[derive(Clone)]
pub struct RecordValue {
    keys: Arc<Keys>,
    values: Arc<Vec<Lazy>>,
}

impl RecordValue {
    /// `values` must have one entry per key; missing entries are filled with `null`.
    pub fn new(keys: Arc<Keys>, mut values: Vec<Lazy>) -> Self {
        values.resize(keys.len(), Lazy::Value(Value::Null));
        Self {
            keys,
            values: Arc::new(values),
        }
    }

    pub fn empty() -> Self {
        Self {
            keys: Arc::new(Keys::empty()),
            values: Arc::new(Vec::new()),
        }
    }

    /// Builds a record from name/value pairs; duplicate names are an error.
    pub fn from_pairs<N: Into<Arc<str>>>(
        pairs: impl IntoIterator<Item = (N, Value)>,
    ) -> Result<Self, MError> {
        let (names, values): (Vec<Arc<str>>, Vec<Lazy>) = pairs
            .into_iter()
            .map(|(n, v)| (n.into(), Lazy::Value(v)))
            .unzip();
        Ok(Self::new(Arc::new(Keys::try_new(names)?), values))
    }

    pub(crate) fn from_lazy_pairs(pairs: Vec<(Arc<str>, Lazy)>) -> Result<Self, MError> {
        let (names, values): (Vec<Arc<str>>, Vec<Lazy>) = pairs.into_iter().unzip();
        Ok(Self::new(Arc::new(Keys::try_new(names)?), values))
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn keys(&self) -> &Arc<Keys> {
        &self.keys
    }

    pub fn names(&self) -> &[Arc<str>] {
        self.keys.names()
    }

    pub fn values(&self) -> &[Lazy] {
        &self.values
    }

    pub fn get(&self, name: &str) -> Option<&Lazy> {
        self.keys.position(name).and_then(|i| self.values.get(i))
    }

    /// The resolved value of a field (see [`Lazy::resolved`]); `None` if the field is missing.
    pub fn field(&self, name: &str) -> Option<Result<Value, MError>> {
        self.get(name).map(Lazy::resolved)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Arc<str>, &Lazy)> {
        self.keys.names().iter().zip(self.values.iter())
    }

    /// `self & other`: fields of `other` replace same-named fields of `self`.
    pub fn merge(&self, other: &RecordValue) -> RecordValue {
        if other.is_empty() {
            return self.clone();
        }
        if self.is_empty() {
            return other.clone();
        }
        let mut names: Vec<Arc<str>> = self.keys.names().to_vec();
        let mut values: Vec<Lazy> = self.values.to_vec();
        for (n, v) in other.iter() {
            match self.keys.position(n) {
                Some(i) => {
                    if let Some(slot) = values.get_mut(i) {
                        *slot = v.clone();
                    }
                }
                None => {
                    names.push(n.clone());
                    values.push(v.clone());
                }
            }
        }
        match Keys::new(names) {
            Ok(keys) => RecordValue::new(Arc::new(keys), values),
            Err(_) => self.clone(),
        }
    }
}

impl Drop for RecordValue {
    fn drop(&mut self) {
        if let Some(values) = Arc::get_mut(&mut self.values)
            && !values.is_empty()
        {
            drop::defer(drop::Garbage::Slots(std::mem::take(values)));
        }
    }
}

impl fmt::Debug for RecordValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(&Value::Record(self.clone()), f)
    }
}

// ---------------------------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------------------------

pub(crate) type NativeFn = fn(&Interp, &[Value]) -> Result<Value, MError>;
pub(crate) type DynNativeFn = dyn Fn(&Interp, &[Value]) -> Result<Value, MError> + Send + Sync;

pub(crate) enum NativeImpl {
    Static(NativeFn),
    Dyn(Arc<DynNativeFn>),
}

/// A parameter of a native (library) function.
#[derive(Debug, Clone)]
pub struct NativeParam {
    pub name: Arc<str>,
    pub optional: bool,
}

pub(crate) struct NativeFunction {
    pub(crate) name: Arc<str>,
    pub(crate) params: Arc<[NativeParam]>,
    pub(crate) imp: NativeImpl,
    /// Receive arguments with their metadata (for `Value.Metadata` & co.).
    pub(crate) raw_args: bool,
}

pub(crate) enum FunctionKind {
    Closure { def: Arc<FnDef>, env: Env },
    Native(NativeFunction),
}

/// An M function: a closure over an environment, or a native library function.
#[derive(Clone)]
pub struct FunctionValue(pub(crate) Arc<FunctionKind>);

impl FunctionValue {
    pub(crate) fn closure(def: Arc<FnDef>, env: Env) -> Self {
        Self(Arc::new(FunctionKind::Closure { def, env }))
    }

    pub(crate) fn native(
        name: impl Into<Arc<str>>,
        params: &[(&str, bool)],
        imp: NativeImpl,
    ) -> Self {
        Self(Arc::new(FunctionKind::Native(NativeFunction {
            name: name.into(),
            params: params
                .iter()
                .map(|(n, o)| NativeParam {
                    name: (*n).into(),
                    optional: *o,
                })
                .collect(),
            imp,
            raw_args: false,
        })))
    }

    pub(crate) fn native_raw(name: &str, params: &[(&str, bool)], f: NativeFn) -> Self {
        let mut func = Self::native(name, params, NativeImpl::Static(f));
        if let Some(FunctionKind::Native(n)) = Arc::get_mut(&mut func.0) {
            n.raw_args = true;
        }
        func
    }

    /// The library name of a native function (`None` for closures).
    pub fn name(&self) -> Option<&str> {
        match &*self.0 {
            FunctionKind::Native(n) => Some(&n.name),
            FunctionKind::Closure { .. } => None,
        }
    }

    /// `(required, total)` parameter counts.
    pub fn arity(&self) -> (usize, usize) {
        match &*self.0 {
            FunctionKind::Native(n) => (
                n.params.iter().filter(|p| !p.optional).count(),
                n.params.len(),
            ),
            FunctionKind::Closure { def, .. } => (
                def.params.iter().filter(|p| !p.optional).count(),
                def.params.len(),
            ),
        }
    }

    /// Parameter names and optionality.
    pub fn params(&self) -> Vec<(Arc<str>, bool)> {
        match &*self.0 {
            FunctionKind::Native(n) => n
                .params
                .iter()
                .map(|p| (p.name.clone(), p.optional))
                .collect(),
            FunctionKind::Closure { def, .. } => def
                .params
                .iter()
                .map(|p| (p.name.clone(), p.optional))
                .collect(),
        }
    }

    pub fn ptr_eq(&self, other: &FunctionValue) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for FunctionValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(n) => write!(f, "<function {n}>"),
            None => f.write_str("<function>"),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Binary
// ---------------------------------------------------------------------------------------------

/// A source of bytes for a lazily loaded [`BinaryValue`] (e.g. a blob's `Content`).
pub trait BinarySource: Send + Sync + fmt::Debug {
    /// Loads the bytes. Implementations should fail with [`MError::LimitExceeded`] rather than
    /// read more than `max_bytes`.
    fn load(&self, max_bytes: u64) -> Result<Vec<u8>, MError>;

    /// The size in bytes, if known without loading.
    fn size_hint(&self) -> Option<u64> {
        None
    }
}

#[derive(Debug)]
struct InMemory(Arc<[u8]>);

impl BinarySource for InMemory {
    fn load(&self, _max_bytes: u64) -> Result<Vec<u8>, MError> {
        Ok(self.0.to_vec())
    }

    fn size_hint(&self) -> Option<u64> {
        u64::try_from(self.0.len()).ok()
    }
}

struct BinaryInner {
    source: Arc<dyn BinarySource>,
    cache: OnceLock<Result<Arc<[u8]>, MError>>,
}

/// An M binary value. Contents are loaded on first use and cached.
#[derive(Clone)]
pub struct BinaryValue(Arc<BinaryInner>);

impl BinaryValue {
    pub fn from_bytes(bytes: impl Into<Arc<[u8]>>) -> Self {
        let bytes: Arc<[u8]> = bytes.into();
        let cache = OnceLock::new();
        let _ = cache.set(Ok(bytes.clone()));
        Self(Arc::new(BinaryInner {
            source: Arc::new(InMemory(bytes)),
            cache,
        }))
    }

    pub fn lazy(source: Arc<dyn BinarySource>) -> Self {
        Self(Arc::new(BinaryInner {
            source,
            cache: OnceLock::new(),
        }))
    }

    /// Loads (once) and returns the bytes, failing if they exceed `max_bytes`.
    pub fn bytes(&self, max_bytes: u64) -> Result<Arc<[u8]>, MError> {
        let r = self.0.cache.get_or_init(|| {
            if let Some(size) = self.0.source.size_hint()
                && size > max_bytes
            {
                return Err(MError::limit(LimitKind::Bytes, max_bytes));
            }
            let bytes = self.0.source.load(max_bytes)?;
            if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
                return Err(MError::limit(LimitKind::Bytes, max_bytes));
            }
            Ok(Arc::from(bytes))
        });
        r.clone()
    }

    pub fn is_loaded(&self) -> bool {
        self.0.cache.get().is_some()
    }

    pub fn size_hint(&self) -> Option<u64> {
        match self.0.cache.get() {
            Some(Ok(b)) => u64::try_from(b.len()).ok(),
            _ => self.0.source.size_hint(),
        }
    }

    pub fn ptr_eq(&self, other: &BinaryValue) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for BinaryValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<binary {:?}>", self.0.source)
    }
}
