//! DashCraft's DAX function catalog: signature, parameters, argument count,
//! return type, category and a one-line description for every function the
//! evaluator supports. The data is `catalog.txt` (its header explains the
//! line format); it is parsed once, on first use.

use std::collections::HashMap;
use std::sync::OnceLock;

const SOURCE: &str = include_str!("catalog.txt");

/// What a function returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReturnType {
    /// Depends on the arguments.
    Any,
    Boolean,
    Currency,
    DateTime,
    Integer,
    Number,
    Table,
    Text,
}

impl ReturnType {
    /// `any`, `boolean`, `currency`, `datetime`, `integer`, `number`,
    /// `table` or `text`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            ReturnType::Any => "any",
            ReturnType::Boolean => "boolean",
            ReturnType::Currency => "currency",
            ReturnType::DateTime => "datetime",
            ReturnType::Integer => "integer",
            ReturnType::Number => "number",
            ReturnType::Table => "table",
            ReturnType::Text => "text",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "any" => ReturnType::Any,
            "boolean" => ReturnType::Boolean,
            "currency" => ReturnType::Currency,
            "datetime" => ReturnType::DateTime,
            "integer" => ReturnType::Integer,
            "number" => ReturnType::Number,
            "table" => ReturnType::Table,
            "text" => ReturnType::Text,
            _ => return None,
        })
    }
}

/// One parameter of a [`FunctionInfo`] signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionParam {
    pub name: &'static str,
    /// May be left out.
    pub optional: bool,
    /// Part of a group that may repeat (`[name, expression]...`).
    pub repeating: bool,
    /// Byte range of the name in [`FunctionInfo::signature`].
    pub label: (usize, usize),
}

/// A DAX function of the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionInfo {
    /// Upper-case name (`SUM`, `PERCENTILE.INC`).
    pub name: &'static str,
    /// `SUM(column)`, `IF(logicalTest, valueIfTrue, [valueIfFalse])`.
    pub signature: &'static str,
    pub params: Vec<FunctionParam>,
    /// Fewest arguments accepted.
    pub min_args: usize,
    /// Most arguments accepted; `None` when unlimited.
    pub max_args: Option<usize>,
    pub returns: ReturnType,
    /// The function family (`Text`, `Time intelligence`, …).
    pub category: &'static str,
    /// One sentence, in DashCraft's words.
    pub description: &'static str,
}

impl FunctionInfo {
    /// Whether a call with `count` arguments has an acceptable number of
    /// arguments.
    #[must_use]
    pub fn accepts(&self, count: usize) -> bool {
        count >= self.min_args && self.max_args.is_none_or(|max| count <= max)
    }

    /// The parameter the argument at `index` (0-based) fills: repeating
    /// groups cycle. `None` past the last parameter of a function without
    /// repeating parameters.
    #[must_use]
    pub fn active_parameter(&self, index: usize) -> Option<usize> {
        let Some(first) = self.params.iter().position(|p| p.repeating) else {
            return (index < self.params.len()).then_some(index);
        };
        if index < first {
            return Some(index);
        }
        let group = self
            .params
            .iter()
            .skip(first)
            .take_while(|p| p.repeating)
            .count()
            .max(1);
        Some(first + (index - first) % group)
    }

    /// `SUM(column) → number`.
    #[must_use]
    pub fn label(&self) -> String {
        format!("{} → {}", self.signature, self.returns.as_str())
    }
}

struct Catalog {
    functions: Vec<FunctionInfo>,
    index: HashMap<&'static str, usize>,
    errors: Vec<String>,
}

fn catalog() -> &'static Catalog {
    static CATALOG: OnceLock<Catalog> = OnceLock::new();
    CATALOG.get_or_init(|| parse_catalog(SOURCE))
}

/// Every catalog function, grouped by category in the catalog's order.
#[must_use]
pub fn functions() -> &'static [FunctionInfo] {
    &catalog().functions
}

/// The catalog entry of `name` (any case).
#[must_use]
pub fn function(name: &str) -> Option<&'static FunctionInfo> {
    let c = catalog();
    c.index
        .get(name)
        .or_else(|| c.index.get(name.to_ascii_uppercase().as_str()))
        .and_then(|&i| c.functions.get(i))
}

/// Power BI's model metadata functions (`INFO.TABLES`, `INFO.VIEW.COLUMNS`,
/// …): DAX functions the catalog does not list, returning tables whose
/// columns it does not know.
pub(super) fn is_info_function(name: &str) -> bool {
    name.len() > 5
        && name
            .get(..5)
            .is_some_and(|p| p.eq_ignore_ascii_case("INFO."))
}

/// Lines of `catalog.txt` that do not parse (empty in a correct build; a
/// test checks it).
#[doc(hidden)]
#[must_use]
pub fn catalog_errors() -> &'static [String] {
    &catalog().errors
}

fn parse_catalog(src: &'static str) -> Catalog {
    let mut functions: Vec<FunctionInfo> = Vec::new();
    let mut index = HashMap::new();
    let mut errors = Vec::new();
    let mut category = "";
    for (n, raw) in src.lines().enumerate() {
        let line = raw.trim_end_matches('\r').trim();
        if let Some(title) = line.strip_prefix("## ") {
            category = title.trim();
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match parse_line(line, category) {
            Ok(f) => {
                if index.insert(f.name, functions.len()).is_some() {
                    errors.push(format!("line {}: duplicate function {}", n + 1, f.name));
                } else {
                    functions.push(f);
                }
            }
            Err(e) => errors.push(format!("line {}: {e}", n + 1)),
        }
    }
    Catalog {
        functions,
        index,
        errors,
    }
}

fn parse_line(line: &'static str, category: &'static str) -> Result<FunctionInfo, String> {
    let (head, description) = line.split_once(" | ").ok_or("missing ` | description`")?;
    let (head, returns) = head.rsplit_once(" -> ").ok_or("missing ` -> type`")?;
    let returns = ReturnType::parse(returns.trim()).ok_or("unknown return type")?;
    let (signature, arity) = match head.split_once(" {") {
        Some((sig, rest)) => (
            sig.trim(),
            Some(rest.strip_suffix('}').ok_or("unclosed {")?),
        ),
        None => (head.trim(), None),
    };
    let open = signature.find('(').ok_or("missing (")?;
    let name = signature.get(..open).ok_or("bad name")?;
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '.')
    {
        return Err(format!("bad function name `{name}`"));
    }
    let inner = signature
        .get(open + 1..)
        .and_then(|s| s.strip_suffix(')'))
        .ok_or("signature must end with )")?;
    let mut params = Vec::new();
    let mut min_args = 0;
    let mut unlimited = false;
    for item in split_top(inner)? {
        let mut text = item.trim();
        let repeating = text.ends_with("...");
        text = text.trim_end_matches("...");
        let optional = text.starts_with('[');
        if optional {
            text = text
                .strip_prefix('[')
                .and_then(|t| t.strip_suffix(']'))
                .ok_or_else(|| format!("unbalanced brackets in `{item}`"))?;
        } else {
            min_args += 1;
        }
        unlimited |= repeating;
        let names = flatten(text)?;
        if names.is_empty() {
            return Err(format!("empty parameter in `{item}`"));
        }
        for name in names {
            params.push(FunctionParam {
                name,
                optional,
                repeating,
                label: (0, 0),
            });
        }
    }
    // Label positions: each name, in order, as a whole word of the signature.
    let mut from = open + 1;
    for p in &mut params {
        let start = find_word(signature, p.name, from)
            .ok_or_else(|| format!("parameter `{}` not found", p.name))?;
        p.label = (start, start + p.name.len());
        from = start + p.name.len();
    }
    let mut max_args = (!unlimited).then_some(params.len());
    if let Some(arity) = arity {
        let (lo, hi) = arity.split_once("..").ok_or("arity must be {min..max}")?;
        min_args = lo.parse().map_err(|_| "bad minimum")?;
        max_args = if hi == "*" {
            None
        } else {
            Some(hi.parse().map_err(|_| "bad maximum")?)
        };
    }
    if description.trim().is_empty() {
        return Err("empty description".into());
    }
    Ok(FunctionInfo {
        name,
        signature,
        params,
        min_args,
        max_args,
        returns,
        category,
        description: description.trim(),
    })
}

/// Splits at the commas outside brackets.
fn split_top(s: &'static str) -> Result<Vec<&'static str>, String> {
    let mut out = Vec::new();
    if s.trim().is_empty() {
        return Ok(out);
    }
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in s.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth = depth.checked_sub(1).ok_or("unbalanced ]")?,
            ',' if depth == 0 => {
                out.push(s.get(start..i).unwrap_or_default());
                start = i + 1;
            }
            _ => {}
        }
    }
    if depth != 0 {
        return Err("unbalanced [".into());
    }
    out.push(s.get(start..).unwrap_or_default());
    Ok(out)
}

/// The parameter names of a (possibly nested) group.
fn flatten(s: &'static str) -> Result<Vec<&'static str>, String> {
    let mut out = Vec::new();
    for item in split_top(s)? {
        let t = item.trim().trim_end_matches("...");
        let t = t
            .strip_prefix('[')
            .and_then(|t| t.strip_suffix(']'))
            .unwrap_or(t);
        if t.contains(',') || t.starts_with('[') {
            out.extend(flatten(t)?);
        } else if t.chars().all(|c| c.is_ascii_alphanumeric()) && !t.is_empty() {
            out.push(t);
        } else {
            return Err(format!("bad parameter name `{t}`"));
        }
    }
    Ok(out)
}

fn find_word(haystack: &str, word: &str, from: usize) -> Option<usize> {
    let mut at = from;
    while let Some(i) = haystack.get(at..).and_then(|h| h.find(word)) {
        let start = at + i;
        let end = start + word.len();
        let before = haystack.get(..start).and_then(|h| h.chars().next_back());
        let after = haystack.get(end..).and_then(|h| h.chars().next());
        if !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
        {
            return Some(start);
        }
        at = end;
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::eval::{is_supported_function, returns_table, supported_functions};

    #[test]
    fn catalog_parses() {
        assert!(catalog_errors().is_empty(), "{:#?}", catalog_errors());
    }

    #[test]
    fn catalog_lists_every_supported_function() {
        let supported: BTreeSet<&str> = supported_functions().into_iter().collect();
        let listed: BTreeSet<&str> = functions().iter().map(|f| f.name).collect();
        let missing: Vec<_> = supported.difference(&listed).collect();
        let extra: Vec<_> = listed.difference(&supported).collect();
        assert!(missing.is_empty(), "not in catalog.txt: {missing:?}");
        assert!(
            extra.is_empty(),
            "in catalog.txt but not supported: {extra:?}"
        );
    }

    const MODIFIERS: [&str; 5] = [
        "ROLLUPADDISSUBTOTAL",
        "ROLLUPGROUP",
        "ORDERBY",
        "PARTITIONBY",
        "MATCHBY",
    ];

    #[test]
    fn table_functions_return_tables() {
        let wrong: Vec<&str> = functions()
            .iter()
            .filter(|f| is_supported_function(f.name))
            // Argument modifiers: the evaluator handles them as table-shaped
            // arguments, but they are never a value on their own.
            .filter(|f| !MODIFIERS.contains(&f.name))
            .filter(|f| (f.returns == ReturnType::Table) != returns_table(f.name))
            .map(|f| f.name)
            .collect();
        assert!(
            wrong.is_empty(),
            "return type disagrees with the evaluator: {wrong:?}"
        );
    }

    #[test]
    fn signatures_parse_into_parameters_and_argument_counts() {
        let f = function("if").unwrap();
        assert_eq!((f.min_args, f.max_args), (2, Some(3)));
        let names: Vec<&str> = f.params.iter().map(|p| p.name).collect();
        assert_eq!(names, ["logicalTest", "valueIfTrue", "valueIfFalse"]);
        assert!(f.params[2].optional && !f.params[1].optional);
        assert_eq!(f.active_parameter(2), Some(2));
        assert_eq!(f.active_parameter(3), None);
        assert!(f.accepts(2) && f.accepts(3) && !f.accepts(1) && !f.accepts(4));

        // A repeating pair after fixed parameters, then an optional else.
        let f = function("SWITCH").unwrap();
        assert_eq!((f.min_args, f.max_args), (3, None));
        assert_eq!(f.active_parameter(3), Some(3));
        assert_eq!(f.active_parameter(4), Some(4));
        assert_eq!(f.active_parameter(5), Some(3));

        // Nested brackets inside a repeating group.
        let f = function("SELECTCOLUMNS").unwrap();
        assert_eq!((f.min_args, f.max_args), (2, None));
        assert_eq!(f.params.len(), 5);

        // An explicit override.
        let f = function("WINDOW").unwrap();
        assert_eq!((f.min_args, f.max_args), (2, Some(10)));

        let f = function("TODAY").unwrap();
        assert_eq!((f.min_args, f.max_args), (0, Some(0)));
        assert_eq!(f.active_parameter(0), None);
        assert_eq!(f.label(), "TODAY() → datetime");
        assert_eq!(f.category, "Date and time");
    }

    #[test]
    fn parameter_labels_point_into_the_signature() {
        for f in functions() {
            // Label offsets are then the same in bytes, characters and
            // UTF-16 units (`dax.signatureHelp` documents them so).
            assert!(f.signature.is_ascii(), "{}", f.signature);
            for p in &f.params {
                assert_eq!(
                    f.signature.get(p.label.0..p.label.1),
                    Some(p.name),
                    "{}",
                    f.signature
                );
            }
        }
    }

    #[test]
    fn malformed_lines_are_reported_not_panicked() {
        let bad = "## X\nFOO(a -> number | x\nbar(a) -> number | x\nBAZ(a) -> thing | x\n\
                   QUX([a) -> number | x\nOK(a) -> number | fine\nOK(b) -> number | dup\n";
        let c = parse_catalog(bad);
        assert_eq!(c.functions.len(), 1);
        assert_eq!(c.errors.len(), 5, "{:#?}", c.errors);
    }
}
