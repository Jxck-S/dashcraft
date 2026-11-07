//! The catalog of TMDL keywords: which keywords Microsoft's TMDL reader accepts in each context,
//! in which forms, from which compatibility level, and what values they take.
//!
//! `catalog.txt` is generated from the ground-truth recording
//! `reference/groundtruth/recordings/tmdl-catalog.json` (`DASHCRAFT_BLESS=1 cargo test -p
//! pbi-tmdl --test groundtruth`), which probed every keyword in five forms in every context with
//! the TOM library's TMDL reader. It holds keywords, forms, levels and value types only.

use std::sync::OnceLock;

const CATALOG_TXT: &str = include_str!("catalog.txt");

/// How a keyword is written on its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Form {
    /// `keyword name`
    Named,
    /// `keyword`
    Bare,
    /// `keyword: value`
    Colon,
    /// `keyword = value`
    Equals,
    /// `keyword name = value`
    NamedEquals,
}

impl Form {
    pub const ALL: [Form; 5] = [
        Form::Named,
        Form::Bare,
        Form::Colon,
        Form::Equals,
        Form::NamedEquals,
    ];

    /// The form as a pattern (`keyword <name> = <value>`).
    pub fn pattern(self, keyword: &str) -> String {
        match self {
            Form::Named => format!("{keyword} <name>"),
            Form::Bare => keyword.to_owned(),
            Form::Colon => format!("{keyword}: <value>"),
            Form::Equals => format!("{keyword} = <value>"),
            Form::NamedEquals => format!("{keyword} <name> = <value>"),
        }
    }

    fn index(self) -> usize {
        match self {
            Form::Named => 0,
            Form::Bare => 1,
            Form::Colon => 2,
            Form::Equals => 3,
            Form::NamedEquals => 4,
        }
    }
}

/// Whether a form is accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    No,
    /// At every compatibility level.
    Yes,
    /// From this compatibility level.
    From(u32),
    /// Only above this compatibility level (newer than the newest level recorded).
    Above(u32),
}

impl Support {
    pub fn accepted(self) -> bool {
        self != Support::No
    }

    /// Whether the form is accepted at compatibility level `level`.
    pub fn accepted_at(self, level: u32) -> bool {
        match self {
            Support::No => false,
            Support::Yes => true,
            Support::From(n) => level >= n,
            Support::Above(n) => level > n,
        }
    }
}

/// Whether a keyword is an object (it can have child lines) or a property (it has a value).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeywordKind {
    Object,
    Property,
    /// A property in the `keyword: value` form, an object otherwise.
    Both,
}

/// What a value must look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueType {
    /// No value.
    None,
    /// Any non-empty text.
    Text,
    /// `true` or `false`.
    Bool,
    /// A 32-bit integer.
    Int,
    /// A 64-bit integer.
    Long,
    /// A number.
    Number,
    /// A value of the TOM enumeration: a name, names separated by commas, or an integer.
    Enum(&'static str),
    /// The name or path of another object.
    Ref,
    /// A JSON object.
    Json,
    /// XML or JSON, as the `contentType` property says.
    Content,
    /// An expression (DAX, M or any text): not checked.
    Expr,
}

impl ValueType {
    fn parse(s: &'static str) -> Option<Self> {
        Some(match s {
            "-" => ValueType::None,
            "text" => ValueType::Text,
            "bool" => ValueType::Bool,
            "int" => ValueType::Int,
            "long" => ValueType::Long,
            "number" => ValueType::Number,
            "ref" => ValueType::Ref,
            "json" => ValueType::Json,
            "content" => ValueType::Content,
            "expr" => ValueType::Expr,
            _ => ValueType::Enum(s.strip_prefix("enum:")?),
        })
    }

    /// The type as `catalog.txt` writes it (`enum:DataType`).
    pub fn as_str(self) -> String {
        match self {
            ValueType::None => "-".to_owned(),
            ValueType::Text => "text".to_owned(),
            ValueType::Bool => "bool".to_owned(),
            ValueType::Int => "int".to_owned(),
            ValueType::Long => "long".to_owned(),
            ValueType::Number => "number".to_owned(),
            ValueType::Enum(t) => format!("enum:{t}"),
            ValueType::Ref => "ref".to_owned(),
            ValueType::Json => "json".to_owned(),
            ValueType::Content => "content".to_owned(),
            ValueType::Expr => "expr".to_owned(),
        }
    }

    /// How the type is described to users (`true or false`).
    pub fn describe(self) -> String {
        match self {
            ValueType::None => "no value".to_owned(),
            ValueType::Text => "text".to_owned(),
            ValueType::Bool => "true or false".to_owned(),
            ValueType::Int | ValueType::Long => "an integer".to_owned(),
            ValueType::Number => "a number".to_owned(),
            ValueType::Enum(t) => format!("a {t} value"),
            ValueType::Ref => "the name of an object".to_owned(),
            ValueType::Json => "a JSON object".to_owned(),
            ValueType::Content => "XML or JSON content".to_owned(),
            ValueType::Expr => "an expression".to_owned(),
        }
    }
}

/// A keyword of a context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keyword {
    pub keyword: &'static str,
    pub kind: KeywordKind,
    /// The value of `keyword: value` (and of `keyword = value` for a property).
    pub value: ValueType,
    /// The value of `keyword = value` and `keyword name = value` for an object.
    pub default: ValueType,
    forms: [Support; 5],
}

impl Keyword {
    pub fn support(&self, form: Form) -> Support {
        self.forms.get(form.index()).copied().unwrap_or(Support::No)
    }

    /// The accepted forms.
    pub fn forms(&self) -> impl Iterator<Item = Form> + '_ {
        Form::ALL
            .into_iter()
            .filter(|f| self.support(*f).accepted())
    }

    /// Whether the keyword is an object when written in `form`.
    pub fn is_object(&self, form: Form) -> bool {
        match form {
            Form::Named | Form::NamedEquals => true,
            Form::Colon => false,
            Form::Bare | Form::Equals => self.kind != KeywordKind::Property,
        }
    }

    /// The value type of `form`.
    pub fn value_type(&self, form: Form) -> ValueType {
        match form {
            Form::Named | Form::Bare => ValueType::None,
            Form::Colon => self.value,
            Form::Equals | Form::NamedEquals if self.kind == KeywordKind::Property => self.value,
            Form::Equals | Form::NamedEquals => self.default,
        }
    }

    /// The lowest compatibility level an accepted form needs, if every accepted form needs one.
    pub fn min_level(&self) -> Option<u32> {
        let mut min: Option<u32> = None;
        for s in self.forms {
            let level = match s {
                Support::No => continue,
                Support::Yes => return None,
                Support::From(n) => n,
                Support::Above(n) => n.saturating_add(1),
            };
            min = Some(min.map_or(level, |m| m.min(level)));
        }
        min
    }
}

/// The child lines of an object type (or the top level of a file, `root`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Context {
    /// `root`, an object keyword (`table`), or a variant: `calculatedColumn` (`column X =
    /// expression`), `calculatedTableColumn` (a column of a calculated table),
    /// `calculatedPartition` and `calculationGroupPartition` (`partition X = calculated` and
    /// `= calculationGroup`).
    pub name: &'static str,
    /// The compatibility level the object type itself needs.
    pub level: Option<u32>,
    /// Sorted by keyword.
    pub keywords: Vec<Keyword>,
}

impl Context {
    /// The keyword (case-insensitive, as TMDL is read).
    pub fn keyword(&self, keyword: &str) -> Option<&Keyword> {
        self.keywords
            .iter()
            .find(|k| k.keyword.eq_ignore_ascii_case(keyword))
    }
}

/// The keyword catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    /// The version of the TOM library the catalog was recorded with.
    pub oracle_version: &'static str,
    pub contexts: Vec<Context>,
    /// Enumeration → (value names, integer values the reader accepts).
    enums: Vec<(&'static str, Vec<&'static str>, Vec<i64>)>,
}

impl Catalog {
    /// The context named `name` (case-insensitive).
    pub fn context(&self, name: &str) -> Option<&Context> {
        self.contexts
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// The names of an enumeration's values as TMDL writes them (`int64`, `dateTime`).
    pub fn enum_values(&self, ty: &str) -> Option<&[&'static str]> {
        self.enums
            .iter()
            .find(|(t, ..)| *t == ty)
            .map(|(_, v, _)| v.as_slice())
    }

    /// The integers (of −1 … 20) the reader accepted for an enumeration: all of them for most
    /// enumerations, only the defined values for a few (`ColumnType`, `RelationshipType`).
    pub fn enum_numbers(&self, ty: &str) -> Option<&[i64]> {
        self.enums
            .iter()
            .find(|(t, ..)| *t == ty)
            .map(|(_, _, n)| n.as_slice())
    }

    /// The contexts that accept `keyword`.
    pub fn contexts_of<'a>(&'a self, keyword: &'a str) -> impl Iterator<Item = &'a Context> + 'a {
        self.contexts
            .iter()
            .filter(move |c| c.keyword(keyword).is_some())
    }

    /// Every keyword of every context, once, sorted.
    pub fn all_keywords(&self) -> Vec<&'static str> {
        let mut all: Vec<&'static str> = self
            .contexts
            .iter()
            .flat_map(|c| c.keywords.iter().map(|k| k.keyword))
            .collect();
        all.sort_unstable();
        all.dedup();
        all
    }
}

/// The catalog (parsed once).
pub fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| parse(CATALOG_TXT))
}

fn parse(text: &'static str) -> Catalog {
    let mut catalog = Catalog {
        oracle_version: "",
        contexts: Vec::new(),
        enums: Vec::new(),
    };
    for line in text.lines() {
        let cells: Vec<&'static str> = line.split('\t').collect();
        match cells.as_slice() {
            ["oracle", version, ..] => catalog.oracle_version = version,
            ["ctx", name, level, ..] => catalog.contexts.push(Context {
                name,
                level: level.parse().ok(),
                keywords: Vec::new(),
            }),
            ["kw", context, rest @ ..] => {
                if let Some(kw) = parse_keyword(rest)
                    && let Some(ctx) = catalog.contexts.iter_mut().find(|c| c.name == *context)
                {
                    ctx.keywords.push(kw);
                }
            }
            ["enum", ty, values @ ..] => {
                let (numbers, names): (Vec<&str>, Vec<&str>) =
                    values.iter().partition(|v| v.parse::<i64>().is_ok());
                let numbers = numbers.iter().filter_map(|v| v.parse().ok()).collect();
                catalog.enums.push((ty, names, numbers));
            }
            _ => {}
        }
    }
    for ctx in &mut catalog.contexts {
        ctx.keywords.sort_by(|a, b| a.keyword.cmp(b.keyword));
    }
    catalog
}

/// `keyword kind value default named bare colon eq namedEq`.
fn parse_keyword(cells: &[&'static str]) -> Option<Keyword> {
    let [keyword, kind, value, default, forms @ ..] = cells else {
        return None;
    };
    let kind = match *kind {
        "object" => KeywordKind::Object,
        "property" => KeywordKind::Property,
        "both" => KeywordKind::Both,
        _ => return None,
    };
    let mut parsed = [Support::No; 5];
    for (slot, cell) in parsed.iter_mut().zip(forms) {
        *slot = match *cell {
            "-" => Support::No,
            "y" => Support::Yes,
            c => match c.strip_prefix('>') {
                Some(n) => Support::Above(n.parse().ok()?),
                None => Support::From(c.parse().ok()?),
            },
        };
    }
    Some(Keyword {
        keyword,
        kind,
        value: ValueType::parse(value)?,
        default: ValueType::parse(default)?,
        forms: parsed,
    })
}
