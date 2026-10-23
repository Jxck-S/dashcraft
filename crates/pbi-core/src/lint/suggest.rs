//! "Did you mean …" suggestions: case-insensitive matches, small edit
//! distances and word-prefix matches (`Category` → `Category Name`).

use pbi_model::{Model, Table, dax_column_ref};

/// Names longer than this are not compared (bounds the DP table).
const MAX_LEN: usize = 256;

/// Case-insensitive Levenshtein distance over chars; `None` if either side
/// is longer than [`MAX_LEN`].
pub(crate) fn edit_distance(a: &str, b: &str) -> Option<usize> {
    let a: Vec<char> = a.chars().flat_map(char::to_lowercase).collect();
    let b: Vec<char> = b.chars().flat_map(char::to_lowercase).collect();
    if a.len() > MAX_LEN || b.len() > MAX_LEN {
        return None;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            cur[j + 1] = (prev[j] + cost).min(prev[j + 1] + 1).min(cur[j] + 1);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev.last().copied()
}

/// `long` starts or ends with `short` at a word boundary (case-insensitive),
/// e.g. `Category Name` / `Category`.
fn word_affix(short: &str, long: &str) -> bool {
    let short = short.to_lowercase();
    let long = long.to_lowercase();
    if short.chars().count() < 4 || short.len() >= long.len() {
        return false;
    }
    let boundary = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
    if let Some(rest) = long.strip_prefix(&short) {
        return boundary(rest.chars().next());
    }
    if let Some(rest) = long.strip_suffix(&short) {
        return boundary(rest.chars().last());
    }
    false
}

/// Score of `candidate` as a correction for `name` (lower is better), or
/// `None` if it is not plausible.
fn score(name: &str, candidate: &str) -> Option<usize> {
    let d = edit_distance(name, candidate)?;
    if d == 0 {
        return Some(0);
    }
    let len = name.chars().count().max(candidate.chars().count());
    let threshold = ((len + 2) / 4).max(1);
    if d <= threshold || word_affix(name, candidate) || word_affix(candidate, name) {
        Some(d)
    } else {
        None
    }
}

/// The most plausible correction for `name` among `candidates`.
pub(crate) fn closest<'a>(
    name: &str,
    candidates: impl IntoIterator<Item = &'a str>,
) -> Option<&'a str> {
    candidates
        .into_iter()
        .filter(|c| *c != name)
        .filter_map(|c| score(name, c).map(|s| (s, c)))
        .min_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)))
        .map(|(_, c)| c)
}

/// Suggestion for a missing table.
pub(crate) fn table(model: &Model, name: &str) -> Option<String> {
    closest(name, model.tables.iter().map(|t| t.name.as_str()))
        .map(|t| format!("did you mean table `{t}`?"))
}

fn field_names(t: &Table) -> impl Iterator<Item = &str> {
    t.columns
        .iter()
        .map(|c| c.name.as_str())
        .chain(t.measures.iter().map(|m| m.name.as_str()))
}

/// Suggestion for a missing column/measure `name` of `table`: a similar
/// field of the same table, the same name on another table, or a similar
/// field anywhere.
pub(crate) fn field(model: &Model, table: &str, name: &str) -> Option<String> {
    if let Some(t) = model.table(table)
        && let Some(c) = closest(name, field_names(t))
    {
        return Some(format!("did you mean `{}`?", dax_column_ref(&t.name, c)));
    }
    let elsewhere: Vec<&Table> = model
        .tables
        .iter()
        .filter(|t| t.column(name).is_some() || t.measure(name).is_some())
        .collect();
    if let [t, ..] = elsewhere.as_slice() {
        let field = t
            .column(name)
            .map(|c| c.name.as_str())
            .or_else(|| t.measure(name).map(|m| m.name.as_str()))
            .unwrap_or(name);
        return Some(format!(
            "`{name}` exists on table `{}`: `{}`",
            t.name,
            dax_column_ref(&t.name, field)
        ));
    }
    let all = model
        .tables
        .iter()
        .flat_map(|t| field_names(t).map(move |f| (t.name.as_str(), f)));
    let mut best: Option<(usize, &str, &str)> = None;
    for (t, f) in all {
        if let Some(s) = score(name, f)
            && best.is_none_or(|(b, _, _)| s < b)
        {
            best = Some((s, t, f));
        }
    }
    best.map(|(_, t, f)| format!("did you mean `{}`?", dax_column_ref(t, f)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_are_case_insensitive() {
        assert_eq!(edit_distance("Category", "categry"), Some(1));
        assert_eq!(edit_distance("", "abc"), Some(3));
        assert_eq!(edit_distance(&"x".repeat(300), "x"), None);
    }

    #[test]
    fn closest_prefers_small_edits_and_word_prefixes() {
        let names = ["Category Name", "Brand", "SKU", "StoreKey"];
        assert_eq!(closest("Category", names), Some("Category Name"));
        assert_eq!(closest("Brnd", names), Some("Brand"));
        assert_eq!(closest("storekey", names), Some("StoreKey"));
        assert_eq!(closest("Supplier", names), None);
        assert_eq!(closest("Cat", ["Category Name"]), None);
        assert_eq!(
            closest("Returns count", ["Returns", "Net Returns"]),
            Some("Returns")
        );
    }
}
