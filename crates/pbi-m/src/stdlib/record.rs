//! `Record.*` functions.

use std::sync::Arc;

use super::args::{arg, items, names_arg, req_list, req_record, req_text};
use super::{Builder, MissingField};
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::{Keys, Lazy, ListValue, RecordValue, TableValue, Value};

pub(super) fn register(b: &mut Builder) {
    b.func("Record.Field", &["record", "field"], field);
    b.func(
        "Record.FieldOrDefault",
        &["record", "field", "?defaultValue"],
        field_or_default,
    );
    b.func("Record.FieldNames", &["record"], field_names);
    b.func("Record.FieldValues", &["record"], field_values);
    b.func("Record.FieldCount", &["record"], field_count);
    b.func("Record.HasFields", &["record", "fields"], has_fields);
    b.func(
        "Record.AddField",
        &["record", "fieldName", "value", "?delayed"],
        add_field,
    );
    b.func(
        "Record.RemoveFields",
        &["record", "fields", "?missingField"],
        remove_fields,
    );
    b.func(
        "Record.SelectFields",
        &["record", "fields", "?missingField"],
        select_fields,
    );
    b.func(
        "Record.RenameFields",
        &["record", "renames", "?missingField"],
        rename_fields,
    );
    b.func("Record.Combine", &["records"], combine);
    b.func("Record.ToTable", &["record"], to_table);
    b.func("Record.ToList", &["record"], to_list);
    b.func("Record.FromList", &["list", "fields"], from_list);
    b.func("Record.FromTable", &["table"], from_table);
}

fn missing_field(name: &str) -> MError {
    MError::expression(format!("The field '{name}' of the record wasn't found."))
}

fn field(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let r = req_record(args, 0)?;
    let name = req_text(args, 1)?;
    r.get(name).ok_or_else(|| missing_field(name))?.force(it)
}

fn field_or_default(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let Value::Record(r) = arg(args, 0) else {
        return Ok(arg(args, 2).clone());
    };
    let name = req_text(args, 1)?;
    match r.get(name) {
        Some(v) => v.force(it),
        None => Ok(arg(args, 2).clone()),
    }
}

fn field_names(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::list(
        req_record(args, 0)?
            .names()
            .iter()
            .map(|n| Value::Text(n.clone())),
    ))
}

fn field_values(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::List(ListValue::new(
        req_record(args, 0)?.values().to_vec(),
    )))
}

fn field_count(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Number(req_record(args, 0)?.len() as f64))
}

fn has_fields(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let r = req_record(args, 0)?;
    let names = names_arg(it, arg(args, 1))?;
    Ok(Value::Logical(names.iter().all(|n| r.get(n).is_some())))
}

fn rebuild(pairs: Vec<(Arc<str>, Lazy)>) -> Result<Value, MError> {
    Ok(Value::Record(RecordValue::from_lazy_pairs(pairs)?))
}

fn pairs_of(r: &RecordValue) -> Vec<(Arc<str>, Lazy)> {
    r.iter().map(|(n, v)| (n.clone(), v.clone())).collect()
}

fn add_field(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let r = req_record(args, 0)?;
    let name = req_text(args, 1)?;
    let mut pairs = pairs_of(r);
    pairs.push((Arc::from(name), Lazy::Value(arg(args, 2).clone())));
    rebuild(pairs)
}

fn missing_mode(args: &[Value], i: usize) -> Result<MissingField, MError> {
    MissingField::from_arg(arg(args, i))
}

fn remove_fields(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let r = req_record(args, 0)?;
    let names = names_arg(it, arg(args, 1))?;
    if missing_mode(args, 2)? == MissingField::Error
        && let Some(n) = names.iter().find(|n| r.get(n).is_none())
    {
        return Err(missing_field(n));
    }
    rebuild(
        pairs_of(r)
            .into_iter()
            .filter(|(n, _)| !names.iter().any(|x| x.as_str() == &**n))
            .collect(),
    )
}

fn select_fields(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let r = req_record(args, 0)?;
    let names = names_arg(it, arg(args, 1))?;
    let mode = missing_mode(args, 2)?;
    let mut pairs = Vec::with_capacity(names.len());
    for n in names {
        match r.get(&n) {
            Some(v) => pairs.push((Arc::from(n), v.clone())),
            None => match mode {
                MissingField::Error => return Err(missing_field(&n)),
                MissingField::Ignore => {}
                MissingField::UseNull => pairs.push((Arc::from(n), Lazy::Value(Value::Null))),
            },
        }
    }
    rebuild(pairs)
}

fn rename_fields(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let r = req_record(args, 0)?;
    let mode = missing_mode(args, 2)?;
    let mut pairs = pairs_of(r);
    let list = match arg(args, 1) {
        Value::List(l) => items(it, l)?,
        other => return Err(convert_error(other, "List")),
    };
    let renames: Vec<Vec<Value>> = if list.first().is_some_and(|x| !matches!(x, Value::List(_))) {
        vec![list]
    } else {
        list.iter()
            .map(|x| match x {
                Value::List(l) => items(it, l),
                other => Err(convert_error(other, "List")),
            })
            .collect::<Result<_, _>>()?
    };
    for pair in renames {
        let [Value::Text(old), Value::Text(new)] = pair.as_slice() else {
            return Err(MError::expression(
                "Record.RenameFields expects {old, new} pairs.",
            ));
        };
        match r.keys().position(old) {
            Some(i) => {
                if let Some(p) = pairs.get_mut(i) {
                    p.0 = new.clone();
                }
            }
            None => match mode {
                MissingField::Error => return Err(missing_field(old)),
                MissingField::Ignore => {}
                MissingField::UseNull => pairs.push((new.clone(), Lazy::Value(Value::Null))),
            },
        }
    }
    rebuild(pairs)
}

fn combine(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut acc = RecordValue::empty();
    for v in items(it, req_list(args, 0)?)? {
        match v {
            Value::Record(r) => acc = acc.merge(&r),
            other => return Err(convert_error(&other, "Record")),
        }
    }
    Ok(Value::Record(acc))
}

fn to_table(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let r = req_record(args, 0)?;
    let rows = r
        .iter()
        .map(|(n, v)| vec![Lazy::Value(Value::Text(n.clone())), v.clone()])
        .collect();
    Ok(Value::Table(TableValue::from_rows(
        vec![Arc::from("Name"), Arc::from("Value")],
        rows,
    )?))
}

fn to_list(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    field_values(it, args)
}

fn from_list(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let values = req_list(args, 0)?;
    let names: Vec<Arc<str>> = match arg(args, 1) {
        Value::Type(t) => match t.kind() {
            crate::value::TypeKind::Record(rt) => {
                rt.fields.iter().map(|f| f.name.clone()).collect()
            }
            _ => return Err(convert_error(arg(args, 1), "List")),
        },
        other => names_arg(it, other)?.into_iter().map(Arc::from).collect(),
    };
    if names.len() != values.len() {
        return Err(MError::expression(
            "The number of field names doesn't match the number of values.",
        ));
    }
    let keys = Keys::try_new(names)?;
    Ok(Value::Record(RecordValue::new(
        Arc::new(keys),
        values.items().to_vec(),
    )))
}

fn from_table(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let t = super::args::req_table(args, 0)?;
    let (Some(names), Some(values)) = (t.column_by_name("Name"), t.column_by_name("Value")) else {
        return Err(super::missing_column("Name"));
    };
    let mut pairs = Vec::with_capacity(names.len());
    for (n, v) in names.iter().zip(values) {
        let n = n.force(it)?;
        let Value::Text(n) = n.plain() else {
            return Err(convert_error(&n, "Text"));
        };
        pairs.push((n.clone(), v.clone()));
    }
    rebuild(pairs)
}
