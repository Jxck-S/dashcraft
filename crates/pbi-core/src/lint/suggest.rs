//! "Did you mean …" suggestions for lint findings, on top of
//! [`pbi_model::closest_name`].

pub(crate) use pbi_model::closest_name as closest;
use pbi_model::{Model, Table, dax_column_ref, name_match_score as score};

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
