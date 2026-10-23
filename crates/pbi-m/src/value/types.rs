//! M type values (`type text`, `type nullable number`, `type table [A = text]`, `Int64.Type`).

use std::fmt;
use std::sync::Arc;

use crate::ast::PrimitiveType;
use crate::value::{Value, ValueKind};

/// A numeric or textual facet carried by library type constants such as `Int64.Type`. Facets do
/// not change type conformance (`1.5 is Int64.Type` is `true`), but they drive conversions
/// (`Table.TransformColumnTypes`) and the Arrow type of a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Facet {
    Int64,
    Int32,
    Int16,
    Int8,
    Byte,
    Single,
    Double,
    Decimal,
    Currency,
    Percentage,
    Character,
    Guid,
    Uri,
    Password,
}

impl Facet {
    /// The library constant name, e.g. `Int64.Type`.
    pub const fn type_name(self) -> &'static str {
        match self {
            Facet::Int64 => "Int64.Type",
            Facet::Int32 => "Int32.Type",
            Facet::Int16 => "Int16.Type",
            Facet::Int8 => "Int8.Type",
            Facet::Byte => "Byte.Type",
            Facet::Single => "Single.Type",
            Facet::Double => "Double.Type",
            Facet::Decimal => "Decimal.Type",
            Facet::Currency => "Currency.Type",
            Facet::Percentage => "Percentage.Type",
            Facet::Character => "Character.Type",
            Facet::Guid => "Guid.Type",
            Facet::Uri => "Uri.Type",
            Facet::Password => "Password.Type",
        }
    }

    pub const fn base(self) -> PrimitiveType {
        match self {
            Facet::Character | Facet::Guid | Facet::Uri | Facet::Password => PrimitiveType::Text,
            _ => PrimitiveType::Number,
        }
    }

    pub const fn is_integer(self) -> bool {
        matches!(
            self,
            Facet::Int64 | Facet::Int32 | Facet::Int16 | Facet::Int8 | Facet::Byte
        )
    }

    pub const ALL: [Facet; 14] = [
        Facet::Int64,
        Facet::Int32,
        Facet::Int16,
        Facet::Int8,
        Facet::Byte,
        Facet::Single,
        Facet::Double,
        Facet::Decimal,
        Facet::Currency,
        Facet::Percentage,
        Facet::Character,
        Facet::Guid,
        Facet::Uri,
        Facet::Password,
    ];
}

#[derive(Debug, Clone)]
pub struct FieldType {
    pub name: Arc<str>,
    pub ty: TypeValue,
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct RecordType {
    pub fields: Vec<FieldType>,
    pub open: bool,
}

#[derive(Debug, Clone)]
pub struct TableType {
    pub columns: Vec<(Arc<str>, TypeValue)>,
}

#[derive(Debug, Clone)]
pub struct ParamType {
    pub name: Arc<str>,
    pub ty: TypeValue,
    pub optional: bool,
}

#[derive(Debug, Clone)]
pub struct FunctionType {
    pub params: Vec<ParamType>,
    pub ret: TypeValue,
}

#[derive(Debug, Clone)]
pub enum TypeKind {
    /// `any`, `text`, `number`, `list`, `table`, ... (without further structure).
    Primitive(PrimitiveType),
    /// `{T}`
    List(TypeValue),
    /// `[a = T, ...]`
    Record(RecordType),
    /// `table [a = T]`
    Table(TableType),
    /// `function (x as T) as U`
    Function(FunctionType),
}

#[derive(Debug, Clone)]
struct TypeDef {
    kind: TypeKind,
    nullable: bool,
    facet: Option<Facet>,
}

/// An M type value.
#[derive(Clone)]
pub struct TypeValue(Arc<TypeDef>);

impl Drop for TypeValue {
    fn drop(&mut self) {
        if let Some(def) = Arc::get_mut(&mut self.0)
            && !matches!(def.kind, TypeKind::Primitive(_))
        {
            let kind = std::mem::replace(&mut def.kind, TypeKind::Primitive(PrimitiveType::Any));
            crate::value::drop::defer(crate::value::drop::Garbage::Type(kind));
        }
    }
}

/// Maximum nesting followed when comparing or displaying types.
const MAX_TYPE_DEPTH: usize = 64;

impl TypeValue {
    pub fn new(kind: TypeKind) -> Self {
        Self(Arc::new(TypeDef {
            kind,
            nullable: false,
            facet: None,
        }))
    }

    pub fn primitive(p: PrimitiveType) -> Self {
        Self::new(TypeKind::Primitive(p))
    }

    pub fn any() -> Self {
        Self::primitive(PrimitiveType::Any)
    }

    pub fn text() -> Self {
        Self::primitive(PrimitiveType::Text)
    }

    pub fn number() -> Self {
        Self::primitive(PrimitiveType::Number)
    }

    /// A library facet type such as `Int64.Type`.
    pub fn facet(facet: Facet) -> Self {
        Self(Arc::new(TypeDef {
            kind: TypeKind::Primitive(facet.base()),
            nullable: false,
            facet: Some(facet),
        }))
    }

    pub fn table(columns: Vec<(Arc<str>, TypeValue)>) -> Self {
        Self::new(TypeKind::Table(TableType { columns }))
    }

    /// `type nullable T`.
    #[must_use]
    pub fn nullable(&self) -> Self {
        if self.0.nullable {
            return self.clone();
        }
        let mut def = (*self.0).clone();
        def.nullable = true;
        Self(Arc::new(def))
    }

    /// `Type.NonNullable(T)`.
    #[must_use]
    pub fn non_nullable(&self) -> Self {
        if !self.0.nullable {
            return self.clone();
        }
        let mut def = (*self.0).clone();
        def.nullable = false;
        Self(Arc::new(def))
    }

    pub fn kind(&self) -> &TypeKind {
        &self.0.kind
    }

    pub fn facet_kind(&self) -> Option<Facet> {
        self.0.facet
    }

    /// The primitive type this type is based on (`table [...]` → `table`).
    pub fn base(&self) -> PrimitiveType {
        match &self.0.kind {
            TypeKind::Primitive(p) => *p,
            TypeKind::List(_) => PrimitiveType::List,
            TypeKind::Record(_) => PrimitiveType::Record,
            TypeKind::Table(_) => PrimitiveType::Table,
            TypeKind::Function(_) => PrimitiveType::Function,
        }
    }

    /// Whether `null` conforms to this type.
    pub fn is_nullable(&self) -> bool {
        self.0.nullable || matches!(self.base(), PrimitiveType::Any | PrimitiveType::Null)
    }

    /// The column types of a table type.
    pub fn table_columns(&self) -> Option<&[(Arc<str>, TypeValue)]> {
        match &self.0.kind {
            TypeKind::Table(t) => Some(&t.columns),
            _ => None,
        }
    }

    /// M type compatibility as used by `is`, `as`, `Value.Is` and parameter checks: only the
    /// primitive type and nullability are checked (structure and facets are not).
    pub fn conforms(&self, v: &Value) -> bool {
        let kind = v.kind();
        if kind == ValueKind::Null {
            return self.is_nullable();
        }
        primitive_conforms(self.base(), kind)
    }

    pub fn ptr_eq(&self, other: &TypeValue) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }

    /// Display name used in conversion error messages (`Text`, `Number`, `Int64.Type`).
    pub fn message_name(&self) -> String {
        if let Some(f) = self.0.facet {
            return f.type_name().to_owned();
        }
        let n = self.base().name();
        let mut c = n.chars();
        match c.next() {
            Some(first) => {
                let mut s = first.to_uppercase().collect::<String>();
                s.push_str(c.as_str());
                match s.as_str() {
                    "Datetime" => "DateTime".to_owned(),
                    "Datetimezone" => "DateTimeZone".to_owned(),
                    "Anynonnull" => "AnyNonNull".to_owned(),
                    _ => s,
                }
            }
            None => String::new(),
        }
    }

    /// Structural equality (used for `=` on type values). Types nested deeper than an
    /// internal limit compare unequal.
    pub fn same_as(&self, other: &TypeValue) -> bool {
        self.same_as_at(other, 0)
    }

    fn same_as_at(&self, other: &TypeValue, depth: usize) -> bool {
        if self.ptr_eq(other) {
            return true;
        }
        if depth > MAX_TYPE_DEPTH {
            return false;
        }
        let depth = depth + 1;
        if self.0.nullable != other.0.nullable || self.0.facet != other.0.facet {
            return false;
        }
        match (&self.0.kind, &other.0.kind) {
            (TypeKind::Primitive(a), TypeKind::Primitive(b)) => a == b,
            (TypeKind::List(a), TypeKind::List(b)) => a.same_as_at(b, depth),
            (TypeKind::Record(a), TypeKind::Record(b)) => {
                a.open == b.open
                    && a.fields.len() == b.fields.len()
                    && a.fields.iter().zip(&b.fields).all(|(x, y)| {
                        x.name == y.name
                            && x.optional == y.optional
                            && x.ty.same_as_at(&y.ty, depth)
                    })
            }
            (TypeKind::Table(a), TypeKind::Table(b)) => {
                a.columns.len() == b.columns.len()
                    && a.columns
                        .iter()
                        .zip(&b.columns)
                        .all(|(x, y)| x.0 == y.0 && x.1.same_as_at(&y.1, depth))
            }
            (TypeKind::Function(a), TypeKind::Function(b)) => {
                a.ret.same_as_at(&b.ret, depth)
                    && a.params.len() == b.params.len()
                    && a.params
                        .iter()
                        .zip(&b.params)
                        .all(|(x, y)| x.optional == y.optional && x.ty.same_as_at(&y.ty, depth))
            }
            _ => false,
        }
    }
}

pub(crate) fn primitive_conforms(p: PrimitiveType, kind: ValueKind) -> bool {
    match p {
        PrimitiveType::Any => true,
        PrimitiveType::AnyNonNull => kind != ValueKind::Null,
        PrimitiveType::None | PrimitiveType::Action => false,
        PrimitiveType::Null => kind == ValueKind::Null,
        other => kind.primitive() == other,
    }
}

/// The type of a value (`Value.Type`).
pub fn type_of(v: &Value) -> TypeValue {
    match v.plain() {
        Value::Table(t) => t.table_type(),
        Value::Function(f) => TypeValue::new(TypeKind::Function(FunctionType {
            params: f
                .params()
                .into_iter()
                .map(|(name, optional)| ParamType {
                    name,
                    ty: TypeValue::any(),
                    optional,
                })
                .collect(),
            ret: TypeValue::any(),
        })),
        Value::Record(r) => TypeValue::new(TypeKind::Record(RecordType {
            fields: r
                .names()
                .iter()
                .map(|n| FieldType {
                    name: n.clone(),
                    ty: TypeValue::any(),
                    optional: false,
                })
                .collect(),
            open: false,
        })),
        Value::List(_) => TypeValue::new(TypeKind::List(TypeValue::any())),
        other => TypeValue::primitive(other.kind().primitive()),
    }
}

impl fmt::Debug for TypeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "type {self}")
    }
}

impl fmt::Display for TypeValue {
    /// M type syntax without the leading `type` keyword (`nullable text`, `table [a = text]`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_type(f, 0)
    }
}

struct Nested<'a>(&'a TypeValue, usize);

impl fmt::Display for Nested<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.write_type(f, self.1)
    }
}

impl TypeValue {
    fn write_type(&self, f: &mut fmt::Formatter<'_>, depth: usize) -> fmt::Result {
        if depth > MAX_TYPE_DEPTH {
            return f.write_str("...");
        }
        fn nested(t: &TypeValue, depth: usize) -> Nested<'_> {
            Nested(t, depth + 1)
        }
        if let Some(facet) = self.0.facet {
            if self.0.nullable {
                f.write_str("nullable ")?;
            }
            return f.write_str(facet.type_name());
        }
        if self.0.nullable {
            f.write_str("nullable ")?;
        }
        match &self.0.kind {
            TypeKind::Primitive(p) => f.write_str(p.name()),
            TypeKind::List(t) => write!(f, "{{{}}}", nested(t, depth)),
            TypeKind::Record(r) => {
                f.write_str("[")?;
                for (i, field) in r.fields.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    if field.optional {
                        f.write_str("optional ")?;
                    }
                    write!(
                        f,
                        "{} = {}",
                        quote_name(&field.name),
                        nested(&field.ty, depth)
                    )?;
                }
                if r.open {
                    if !r.fields.is_empty() {
                        f.write_str(", ")?;
                    }
                    f.write_str("...")?;
                }
                f.write_str("]")
            }
            TypeKind::Table(t) => {
                f.write_str("table [")?;
                for (i, (name, ty)) in t.columns.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{} = {}", quote_name(name), nested(ty, depth))?;
                }
                f.write_str("]")
            }
            TypeKind::Function(ft) => {
                f.write_str("function (")?;
                for (i, p) in ft.params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    if p.optional {
                        f.write_str("optional ")?;
                    }
                    write!(f, "{} as {}", quote_name(&p.name), nested(&p.ty, depth))?;
                }
                write!(f, ") as {}", nested(&ft.ret, depth))
            }
        }
    }
}

pub(crate) fn quote_name(name: &str) -> String {
    let simple = !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_alphanumeric() || c == '_');
    if simple {
        name.to_owned()
    } else {
        format!("#\"{}\"", name.replace('"', "\"\""))
    }
}
