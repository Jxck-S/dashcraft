//! The catalog of Power BI Desktop's M standard library.
//!
//! `library.txt` lists every name that Desktop's mashup engine has in its standard library (the
//! dotted names of `#shared`): functions with their parameters, parameter types and return type,
//! type constants with their primitive kind, and number, text and logical constants with their
//! value. It is generated from the ground-truth recording
//! `reference/groundtruth/recordings/m-library.json` (`DASHCRAFT_BLESS=1 cargo test -p pbi-m
//! --test library`) and holds names, types, counts and values only: no documentation text.

use std::sync::OnceLock;

const LIBRARY_TXT: &str = include_str!("library.txt");

/// A parameter of a library function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryParam {
    pub name: &'static str,
    /// Desktop's type for the parameter: a primitive kind, possibly `nullable` (`nullable text`).
    pub ty: &'static str,
    pub optional: bool,
}

/// The signature of a library function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryFunction {
    pub params: Vec<LibraryParam>,
    pub required: usize,
    /// The return type's primitive kind (`table`, `nullable number`, `any`).
    pub returns: &'static str,
}

/// What a library name is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibraryKind {
    Function(LibraryFunction),
    /// A type constant (`Int64.Type`) and its primitive kind (`number`).
    Type(&'static str),
    /// A number, text or logical constant and its value as an M literal (`3`, `"GET"`, `#nan`).
    Constant {
        kind: &'static str,
        value: &'static str,
    },
    /// Any other value (`record`, `null`), or a constant whose value depends on the machine
    /// (`Culture.Current`, `TimeZone.Current`).
    Value(&'static str),
}

/// One name of the standard library.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryEntry {
    pub name: &'static str,
    pub kind: LibraryKind,
}

impl LibraryEntry {
    /// Whether DashCraft evaluates this name (the rest of the library is known but unsupported).
    pub fn implemented(&self) -> bool {
        crate::stdlib::library().get(self.name).is_some()
    }

    /// The function's signature as M shows it
    /// (`Text.Upper(text as nullable text, optional culture as nullable text) as nullable text`).
    pub fn signature(&self) -> Option<String> {
        let LibraryKind::Function(f) = &self.kind else {
            return None;
        };
        let params = f
            .params
            .iter()
            .map(|p| {
                format!(
                    "{}{} as {}",
                    if p.optional { "optional " } else { "" },
                    identifier(p.name),
                    p.ty
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        Some(format!("{}({params}) as {}", self.name, f.returns))
    }
}

/// `name`, or `#"name"` when it is not a regular identifier.
fn identifier(name: &str) -> String {
    let regular = name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '.');
    if regular {
        name.to_owned()
    } else {
        format!("#\"{}\"", name.replace('"', "\"\""))
    }
}

/// Every name of Desktop's standard library, sorted by name (ordinal).
pub fn catalog() -> &'static [LibraryEntry] {
    static CATALOG: OnceLock<Vec<LibraryEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let mut entries: Vec<LibraryEntry> = LIBRARY_TXT.lines().filter_map(parse_line).collect();
        entries.sort_by(|a, b| a.name.cmp(b.name));
        entries.dedup_by(|a, b| a.name == b.name);
        entries
    })
}

/// The library entry named exactly `name` (M names are case-sensitive).
pub fn catalog_entry(name: &str) -> Option<&'static LibraryEntry> {
    let c = catalog();
    c.binary_search_by(|e| e.name.cmp(name))
        .ok()
        .and_then(|i| c.get(i))
}

/// The Desktop version the catalog was recorded with (from the header of `library.txt`).
pub fn catalog_desktop_version() -> Option<&'static str> {
    LIBRARY_TXT
        .lines()
        .take_while(|l| l.starts_with('#'))
        .find_map(|l| l.strip_prefix("# desktop "))
        .map(str::trim)
}

fn parse_line(line: &'static str) -> Option<LibraryEntry> {
    if line.starts_with('#') || line.trim().is_empty() {
        return None;
    }
    let mut cols = line.split('\t');
    let name = cols.next()?;
    let kind = cols.next()?;
    let kind = match kind {
        "function" => {
            let required: usize = cols.next()?.parse().ok()?;
            let params_col = cols.next()?;
            let returns = cols.next()?;
            let mut params = Vec::new();
            if !params_col.is_empty() {
                for (i, p) in params_col.split(',').enumerate() {
                    let (pname, ty) = p.trim().split_once(':')?;
                    params.push(LibraryParam {
                        name: pname,
                        ty,
                        optional: i >= required,
                    });
                }
            }
            if required > params.len() {
                return None;
            }
            LibraryKind::Function(LibraryFunction {
                params,
                required,
                returns,
            })
        }
        "type" => LibraryKind::Type(cols.next()?),
        "number" | "text" | "logical" => match cols.next() {
            Some(value) => LibraryKind::Constant { kind, value },
            None => LibraryKind::Value(kind),
        },
        other => LibraryKind::Value(other),
    };
    Some(LibraryEntry { name, kind })
}
