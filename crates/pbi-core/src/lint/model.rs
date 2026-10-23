//! Semantic-model checks: DAX (LINT007/LINT008), relationships
//! (LINT009/LINT010), query groups (LINT011), M references (LINT013) and
//! column references inside the model (LINT017).

use pbi_model::{
    ColumnRef, DataType, ExpressionKind, Model, ObjectId, Origin, PartitionSource, Relationship,
    Severity, Table, dax_column_ref, dax_table_ref,
};

use super::m::Globals;
use super::{ExprSite, Finding, LineBase, Sink, dax, m, rules, suggest};

pub(crate) fn check_model(sink: &mut Sink<'_>, model: &Model) {
    let globals = Globals::new(model);
    for t in &model.tables {
        check_table(sink, model, &globals, t);
    }
    for e in &model.expressions {
        let site = ExprSite {
            what: format!("expression `{}`", e.name),
            expression: &e.expression,
            origin: e.origin.as_ref(),
            line_base: LineBase::Declaration,
            object: e.id,
        };
        if e.kind == ExpressionKind::M {
            m::check(sink, &globals, &site);
        }
        query_group(
            sink,
            model,
            e.query_group.as_deref(),
            &site.what,
            e.origin.as_ref(),
            e.id,
        );
    }
    for role in &model.roles {
        for p in &role.table_permissions {
            let what = format!("role `{}` filter on `{}`", role.name, p.table);
            if model.table(&p.table).is_none() {
                sink.push(
                    Finding::new(
                        rules::DAX_REF,
                        Severity::Error,
                        format!("{what}: unknown table `{}`", dax_table_ref(&p.table)),
                    )
                    .at(role.origin.clone())
                    .on(p.id)
                    .suggest(suggest::table(model, &p.table)),
                );
            }
            if let Some(expr) = &p.filter_expression {
                let site = ExprSite {
                    what,
                    expression: expr,
                    origin: role.origin.as_ref(),
                    line_base: LineBase::Unknown,
                    object: p.id,
                };
                dax::check(sink, model, &site);
            }
        }
    }
    for r in &model.relationships {
        check_relationship(sink, model, r);
    }
}

fn check_table(sink: &mut Sink<'_>, model: &Model, globals: &Globals<'_>, t: &Table) {
    for measure in &t.measures {
        let site = ExprSite {
            what: format!("measure `{}`", dax_column_ref(&t.name, &measure.name)),
            expression: &measure.expression,
            origin: measure.origin.as_ref(),
            line_base: LineBase::Declaration,
            object: measure.id,
        };
        dax::check(sink, model, &site);
    }
    for c in &t.columns {
        if let Some(expr) = c.expression() {
            let site = ExprSite {
                what: format!("calculated column `{}`", dax_column_ref(&t.name, &c.name)),
                expression: expr,
                origin: c.origin.as_ref(),
                line_base: LineBase::Declaration,
                object: c.id,
            };
            dax::check(sink, model, &site);
        }
        if let Some(sort_by) = c.sort_by_column.as_deref()
            && t.column(sort_by).is_none()
        {
            sink.push(
                Finding::new(
                    rules::MODEL_COLUMN_REF,
                    Severity::Error,
                    format!(
                        "column `{}` sorts by `{}`, which does not exist",
                        dax_column_ref(&t.name, &c.name),
                        dax_column_ref(&t.name, sort_by)
                    ),
                )
                .at(c.origin.clone())
                .on(c.id)
                .suggest(column_suggestion(t, sort_by)),
            );
        }
    }
    for h in &t.hierarchies {
        for level in &h.levels {
            if level.column.is_empty() || t.column(&level.column).is_some() {
                continue;
            }
            sink.push(
                Finding::new(
                    rules::MODEL_COLUMN_REF,
                    Severity::Error,
                    format!(
                        "hierarchy `{}` level `{}` uses column `{}`, which does not exist",
                        h.name,
                        level.name,
                        dax_column_ref(&t.name, &level.column)
                    ),
                )
                .at(h.origin.clone())
                .on(level.id)
                .suggest(column_suggestion(t, &level.column)),
            );
        }
    }
    for p in &t.partitions {
        let what = if t.partitions.len() == 1 && p.name == t.name {
            format!("partition of `{}`", t.name)
        } else {
            format!("partition `{}` of `{}`", p.name, t.name)
        };
        let site = |expression| ExprSite {
            what: what.clone(),
            expression,
            origin: p.origin.as_ref(),
            line_base: LineBase::Unknown,
            object: p.id,
        };
        match &p.source {
            PartitionSource::Calculated { expression } => {
                dax::check(sink, model, &site(expression));
            }
            PartitionSource::M { expression } => m::check(sink, globals, &site(expression)),
            _ => {}
        }
        query_group(
            sink,
            model,
            p.query_group.as_deref(),
            &what,
            p.origin.as_ref(),
            p.id,
        );
    }
}

fn column_suggestion(t: &Table, name: &str) -> Option<String> {
    suggest::closest(name, t.columns.iter().map(|c| c.name.as_str()))
        .map(|c| format!("did you mean `{}`?", dax_column_ref(&t.name, c)))
}

fn query_group(
    sink: &mut Sink<'_>,
    model: &Model,
    group: Option<&str>,
    what: &str,
    origin: Option<&Origin>,
    object: ObjectId,
) {
    let Some(group) = group else {
        return;
    };
    if model.query_group(group).is_some() {
        return;
    }
    let suggestion = suggest::closest(group, model.query_groups.iter().map(|g| g.folder.as_str()))
        .map(|g| format!("did you mean `{g}`?"))
        .or_else(|| Some(format!("add `queryGroup '{group}'` to model.tmdl")));
    sink.push(
        Finding::new(
            rules::QUERY_GROUP_REF,
            Severity::Error,
            format!("{what} is in query group `{group}`, which is not defined (deploys fail)"),
        )
        .at(origin.cloned())
        .on(object)
        .suggest(suggestion),
    );
}

fn check_relationship(sink: &mut Sink<'_>, model: &Model, r: &Relationship) {
    let name = if r.name.is_empty() {
        format!(
            "{} → {}",
            dax_column_ref(&r.from.table, &r.from.column),
            dax_column_ref(&r.to.table, &r.to.column)
        )
    } else {
        r.name.clone()
    };
    let mut resolved = Vec::new();
    for (end, side) in [(&r.from, "from"), (&r.to, "to")] {
        match resolve_end(model, end) {
            Ok(dt) => resolved.push(dt),
            Err((message, suggestion)) => sink.push(
                Finding::new(
                    rules::RELATIONSHIP_REF,
                    Severity::Error,
                    format!("relationship `{name}` {side}Column: {message}"),
                )
                .at(r.origin.clone())
                .on(r.id)
                .suggest(suggestion),
            ),
        }
    }
    if let [from, to] = resolved.as_slice()
        && concrete(from)
        && concrete(to)
        && from != to
    {
        sink.push(
            Finding::new(
                rules::RELATIONSHIP_TYPE,
                Severity::Warning,
                format!(
                    "relationship `{name}` joins `{}` ({}) to `{}` ({})",
                    dax_column_ref(&r.from.table, &r.from.column),
                    from.as_str(),
                    dax_column_ref(&r.to.table, &r.to.column),
                    to.as_str()
                ),
            )
            .at(r.origin.clone())
            .on(r.id)
            .suggest(Some("give both key columns the same data type".to_owned())),
        );
    }
}

fn concrete(dt: &DataType) -> bool {
    !matches!(
        dt,
        DataType::Automatic | DataType::Variant | DataType::Unknown(_)
    )
}

type EndError = (String, Option<String>);

fn resolve_end(model: &Model, end: &ColumnRef) -> Result<DataType, EndError> {
    let Some(t) = model.table(&end.table) else {
        return Err((
            format!("table `{}` does not exist", end.table),
            suggest::table(model, &end.table),
        ));
    };
    match t.column(&end.column) {
        Some(c) => Ok(c.data_type.clone()),
        None => Err((
            format!(
                "column `{}` does not exist",
                dax_column_ref(&t.name, &end.column)
            ),
            column_suggestion(t, &end.column),
        )),
    }
}
