//! `List.*` functions.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};

use super::Builder;
use super::args::{arg, catch, count_arg, func_arg, items, num_arg, req_func, req_list};
use super::table::is_ignore_case;
use crate::ast::BinaryOp;
use crate::eval::interp::{Interp, convert_error};
use crate::merror::MError;
use crate::value::ops::{arithmetic, equals_with, hash_value, sort_compare};
use crate::value::{FunctionValue, Lazy, ListValue, Value};

pub(super) fn register(b: &mut Builder) {
    b.func("List.Select", &["list", "selection"], select);
    b.func("List.Transform", &["list", "transform"], transform);
    b.func("List.IsEmpty", &["list"], is_empty);
    b.func("List.RemoveNulls", &["list"], remove_nulls);
    b.func("List.Count", &["list"], count);
    b.func("List.NonNullCount", &["list"], non_null_count);
    b.func(
        "List.Contains",
        &["list", "value", "?equationCriteria"],
        contains,
    );
    b.func(
        "List.ContainsAny",
        &["list", "values", "?equationCriteria"],
        contains_any,
    );
    b.func(
        "List.ContainsAll",
        &["list", "values", "?equationCriteria"],
        contains_all,
    );
    b.func(
        "List.PositionOf",
        &["list", "value", "?occurrence", "?equationCriteria"],
        position_of,
    );
    b.func("List.First", &["list", "?defaultValue"], first);
    b.func("List.Last", &["list", "?defaultValue"], last);
    b.func("List.FirstN", &["list", "countOrCondition"], first_n);
    b.func("List.LastN", &["list", "?countOrCondition"], last_n);
    b.func("List.Skip", &["list", "?countOrCondition"], skip);
    b.func("List.RemoveFirstN", &["list", "?countOrCondition"], skip);
    b.func("List.Range", &["list", "offset", "?count"], range);
    b.func("List.Distinct", &["list", "?equationCriteria"], distinct);
    b.func("List.Sum", &["list", "?precision"], sum);
    b.func("List.Average", &["list", "?precision"], average);
    b.func(
        "List.Max",
        &["list", "?default", "?comparisonCriteria", "?includeNulls"],
        max,
    );
    b.func(
        "List.Min",
        &["list", "?default", "?comparisonCriteria", "?includeNulls"],
        min,
    );
    b.func("List.Combine", &["lists"], combine);
    b.func(
        "List.Accumulate",
        &["list", "seed", "accumulator"],
        accumulate,
    );
    b.func("List.Numbers", &["start", "count", "?increment"], numbers);
    b.func("List.Reverse", &["list"], reverse);
    b.func("List.Sort", &["list", "?comparisonCriteria"], sort);
    b.func("List.AnyTrue", &["list"], any_true);
    b.func("List.AllTrue", &["list"], all_true);
    b.func("List.MatchesAny", &["list", "condition"], matches_any);
    b.func("List.MatchesAll", &["list", "condition"], matches_all);
    b.func("List.Zip", &["lists"], zip);
    b.func("List.Repeat", &["list", "count"], repeat);
    b.func(
        "List.Generate",
        &["initial", "condition", "next", "?selector"],
        generate,
    );
    b.func("List.Buffer", &["list"], identity);
    b.func(
        "List.Difference",
        &["list1", "list2", "?equationCriteria"],
        difference,
    );
    b.func("List.Union", &["lists", "?equationCriteria"], union);
    b.func("List.Intersect", &["lists", "?equationCriteria"], intersect);
    b.func("List.RemoveItems", &["list1", "list2"], remove_items);
    b.func("List.Single", &["list"], single);
    b.func(
        "List.SingleOrDefault",
        &["list", "?default"],
        single_or_default,
    );
}

fn identity(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(arg(args, 0).clone())
}

fn list_of(v: Vec<Value>) -> Value {
    Value::List(ListValue::from_values(v))
}

fn truthy(it: &Interp, f: &FunctionValue, v: Value) -> Result<bool, MError> {
    match it.call(f, &[v])?.plain() {
        Value::Logical(b) => Ok(*b),
        other => Err(convert_error(other, "Logical")),
    }
}

fn select(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let f = req_func(args, 1)?;
    let mut out = Vec::new();
    for v in items(it, req_list(args, 0)?)? {
        if truthy(it, f, v.clone())? {
            out.push(v);
        }
    }
    Ok(list_of(out))
}

fn transform(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let f = req_func(args, 1)?;
    let mut out = Vec::with_capacity(l.len());
    for item in l.items() {
        it.tick()?;
        out.push(match item.force(it) {
            Ok(v) => catch(it.call(f, &[v]))?,
            Err(e) => catch(Err(e))?,
        });
    }
    Ok(Value::List(ListValue::new(out)))
}

fn is_empty(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(req_list(args, 0)?.is_empty()))
}

fn remove_nulls(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(list_of(
        items(it, req_list(args, 0)?)?
            .into_iter()
            .filter(|v| !v.is_null())
            .collect(),
    ))
}

fn count(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Number(req_list(args, 0)?.len() as f64))
}

fn non_null_count(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Number(
        items(it, req_list(args, 0)?)?
            .iter()
            .filter(|v| !v.is_null())
            .count() as f64,
    ))
}

fn index_of(
    it: &Interp,
    list: &[Value],
    v: &Value,
    ignore_case: bool,
) -> Result<Option<usize>, MError> {
    for (i, x) in list.iter().enumerate() {
        if equals_with(it, x, v, ignore_case)? {
            return Ok(Some(i));
        }
    }
    Ok(None)
}

fn contains(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let list = items(it, req_list(args, 0)?)?;
    Ok(Value::Logical(
        index_of(it, &list, arg(args, 1), is_ignore_case(arg(args, 2)))?.is_some(),
    ))
}

fn contains_any(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let list = items(it, req_list(args, 0)?)?;
    let ic = is_ignore_case(arg(args, 2));
    for v in items(it, req_list(args, 1)?)? {
        if index_of(it, &list, &v, ic)?.is_some() {
            return Ok(Value::Logical(true));
        }
    }
    Ok(Value::Logical(false))
}

fn contains_all(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let list = items(it, req_list(args, 0)?)?;
    let ic = is_ignore_case(arg(args, 2));
    for v in items(it, req_list(args, 1)?)? {
        if index_of(it, &list, &v, ic)?.is_none() {
            return Ok(Value::Logical(false));
        }
    }
    Ok(Value::Logical(true))
}

fn position_of(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let list = items(it, req_list(args, 0)?)?;
    let v = arg(args, 1);
    let ic = is_ignore_case(arg(args, 3));
    let mut positions = Vec::new();
    for (i, x) in list.iter().enumerate() {
        if equals_with(it, x, v, ic)? {
            positions.push(i as f64);
        }
    }
    Ok(match num_arg(args, 2)? {
        Some(1.0) => Value::Number(positions.last().copied().unwrap_or(-1.0)),
        Some(2.0) => list_of(positions.into_iter().map(Value::Number).collect()),
        _ => Value::Number(positions.first().copied().unwrap_or(-1.0)),
    })
}

fn first(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    match req_list(args, 0)?.items().first() {
        Some(v) => v.force(it),
        None => Ok(arg(args, 1).clone()),
    }
}

fn last(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    match req_list(args, 0)?.items().last() {
        Some(v) => v.force(it),
        None => Ok(arg(args, 1).clone()),
    }
}

/// Number of leading items selected by a count or condition.
fn leading(
    it: &Interp,
    l: &ListValue,
    v: &Value,
    default: usize,
    from_end: bool,
) -> Result<usize, MError> {
    match v.plain() {
        Value::Null => Ok(default),
        Value::Number(_) => Ok(count_arg(std::slice::from_ref(v), 0)?.unwrap_or(default)),
        Value::Function(f) => {
            let mut n = 0;
            let slots: Box<dyn Iterator<Item = &Lazy>> = if from_end {
                Box::new(l.items().iter().rev())
            } else {
                Box::new(l.items().iter())
            };
            for item in slots {
                it.tick()?;
                if !truthy(it, f, item.force(it)?)? {
                    break;
                }
                n += 1;
            }
            Ok(n)
        }
        other => Err(convert_error(other, "Number")),
    }
}

fn slice(l: &ListValue, from: usize, to: usize) -> Value {
    Value::List(ListValue::new(
        l.items()
            .get(from..to)
            .map(<[Lazy]>::to_vec)
            .unwrap_or_default(),
    ))
}

fn first_n(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let n = leading(it, l, arg(args, 1), 0, false)?.min(l.len());
    Ok(slice(l, 0, n))
}

fn last_n(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let n = leading(it, l, arg(args, 1), 1, true)?.min(l.len());
    Ok(slice(l, l.len() - n, l.len()))
}

fn skip(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let n = leading(it, l, arg(args, 1), 1, false)?.min(l.len());
    Ok(slice(l, n, l.len()))
}

fn range(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let offset = count_arg(args, 1)?.unwrap_or(0).min(l.len());
    let end = match count_arg(args, 2)? {
        Some(c) => offset.saturating_add(c).min(l.len()),
        None => l.len(),
    };
    Ok(slice(l, offset, end))
}

fn dedup(it: &Interp, values: Vec<Value>, ignore_case: bool) -> Result<Vec<Value>, MError> {
    let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut out: Vec<Value> = Vec::new();
    for v in values {
        it.tick()?;
        let mut h = DefaultHasher::new();
        hash_value(&v, ignore_case, &mut h);
        let bucket = buckets.entry(h.finish()).or_default();
        let mut dup = false;
        for &i in bucket.iter() {
            if equals_with(it, &out[i], &v, ignore_case)? {
                dup = true;
                break;
            }
        }
        if !dup {
            bucket.push(out.len());
            out.push(v);
        }
    }
    Ok(out)
}

fn distinct(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let values = items(it, req_list(args, 0)?)?;
    Ok(list_of(dedup(it, values, is_ignore_case(arg(args, 1)))?))
}

fn sum(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut acc: Option<Value> = None;
    for v in items(it, req_list(args, 0)?)? {
        if v.is_null() {
            continue;
        }
        acc = Some(match acc {
            None => match v {
                Value::Number(_) | Value::Duration(_) => v,
                other => return Err(convert_error(&other, "Number")),
            },
            Some(a) => arithmetic(BinaryOp::Add, &a, &v)?,
        });
    }
    Ok(acc.unwrap_or(Value::Null))
}

fn average(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut total = 0.0;
    let mut n = 0usize;
    for v in items(it, req_list(args, 0)?)? {
        match v {
            Value::Null => {}
            Value::Number(x) => {
                total += x;
                n += 1;
            }
            other => return Err(convert_error(&other, "Number")),
        }
    }
    Ok(if n == 0 {
        Value::Null
    } else {
        Value::Number(total / n as f64)
    })
}

fn extreme(it: &Interp, args: &[Value], want: Ordering) -> Result<Value, MError> {
    let key = func_arg(args, 2)?;
    let include_nulls = super::args::bool_arg(args, 3)?.unwrap_or(false);
    let mut best: Option<(Value, Value)> = None;
    for v in items(it, req_list(args, 0)?)? {
        if v.is_null() && !include_nulls {
            continue;
        }
        let k = match key {
            Some(f) => it.call(f, std::slice::from_ref(&v))?,
            None => v.clone(),
        };
        let better = match &best {
            None => true,
            Some((bk, _)) => sort_compare(&k, bk) == want,
        };
        if better {
            best = Some((k, v));
        }
    }
    Ok(best.map_or_else(|| arg(args, 1).clone(), |(_, v)| v))
}

fn max(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    extreme(it, args, Ordering::Greater)
}

fn min(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    extreme(it, args, Ordering::Less)
}

fn combine(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut out = Vec::new();
    for v in items(it, req_list(args, 0)?)? {
        match v {
            Value::List(l) => {
                it.check_rows(out.len() + l.len())?;
                out.extend_from_slice(l.items());
            }
            other => return Err(convert_error(&other, "List")),
        }
    }
    Ok(Value::List(ListValue::new(out)))
}

fn accumulate(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let f = req_func(args, 2)?;
    let mut acc = arg(args, 1).clone();
    for item in req_list(args, 0)?.items() {
        it.tick()?;
        let v = item.force(it)?;
        acc = it.call(f, &[acc, v])?;
    }
    Ok(acc)
}

fn numbers(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let start = num_arg(args, 0)?.unwrap_or(0.0);
    let n = count_arg(args, 1)?.unwrap_or(0);
    let step = num_arg(args, 2)?.unwrap_or(1.0);
    it.check_rows(n)?;
    Ok(list_of(
        (0..n)
            .map(|i| Value::Number(start + step * i as f64))
            .collect(),
    ))
}

fn reverse(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut v = req_list(args, 0)?.items().to_vec();
    v.reverse();
    Ok(Value::List(ListValue::new(v)))
}

fn sort(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut values = items(it, req_list(args, 0)?)?;
    let criteria = arg(args, 1);
    let (key, descending) = match criteria.plain() {
        Value::Null => (None, false),
        Value::Number(n) => (None, *n == 1.0),
        Value::Function(f) if f.arity().1 >= 2 => {
            let error = std::cell::RefCell::new(None);
            values.sort_by(|a, b| {
                if error.borrow().is_some() {
                    return Ordering::Equal;
                }
                match it.call(f, &[a.clone(), b.clone()]) {
                    Ok(r) => r.as_number().map_or(Ordering::Equal, |n| {
                        n.partial_cmp(&0.0).unwrap_or(Ordering::Equal)
                    }),
                    Err(e) => {
                        *error.borrow_mut() = Some(e);
                        Ordering::Equal
                    }
                }
            });
            if let Some(e) = error.into_inner() {
                return Err(e);
            }
            return Ok(list_of(values));
        }
        Value::Function(f) => (Some(f.clone()), false),
        Value::List(l) => {
            let parts = items(it, l)?;
            let f = parts.first().and_then(Value::as_function).cloned();
            (f, parts.get(1).and_then(Value::as_number) == Some(1.0))
        }
        other => return Err(convert_error(other, "Function")),
    };
    let mut keyed: Vec<(Value, Value)> = Vec::with_capacity(values.len());
    for v in values {
        let k = match &key {
            Some(f) => it.call(f, std::slice::from_ref(&v))?,
            None => v.clone(),
        };
        keyed.push((k, v));
    }
    keyed.sort_by(|a, b| {
        let o = sort_compare(&a.0, &b.0);
        if descending { o.reverse() } else { o }
    });
    Ok(list_of(keyed.into_iter().map(|(_, v)| v).collect()))
}

fn logicals(it: &Interp, args: &[Value]) -> Result<Vec<bool>, MError> {
    items(it, req_list(args, 0)?)?
        .into_iter()
        .map(|v| v.as_logical().ok_or_else(|| convert_error(&v, "Logical")))
        .collect()
}

fn any_true(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(logicals(it, args)?.into_iter().any(|b| b)))
}

fn all_true(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    Ok(Value::Logical(logicals(it, args)?.into_iter().all(|b| b)))
}

fn matches_any(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let f = req_func(args, 1)?;
    for v in items(it, req_list(args, 0)?)? {
        if truthy(it, f, v)? {
            return Ok(Value::Logical(true));
        }
    }
    Ok(Value::Logical(false))
}

fn matches_all(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let f = req_func(args, 1)?;
    for v in items(it, req_list(args, 0)?)? {
        if !truthy(it, f, v)? {
            return Ok(Value::Logical(false));
        }
    }
    Ok(Value::Logical(true))
}

fn zip(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let lists: Vec<ListValue> = items(it, req_list(args, 0)?)?
        .into_iter()
        .map(|v| match v {
            Value::List(l) => Ok(l),
            other => Err(convert_error(&other, "List")),
        })
        .collect::<Result<_, _>>()?;
    let n = lists.iter().map(ListValue::len).max().unwrap_or(0);
    Ok(list_of(
        (0..n)
            .map(|i| {
                Value::List(ListValue::new(
                    lists
                        .iter()
                        .map(|l| {
                            l.items()
                                .get(i)
                                .cloned()
                                .unwrap_or(Lazy::Value(Value::Null))
                        })
                        .collect(),
                ))
            })
            .collect(),
    ))
}

fn repeat(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let n = count_arg(args, 1)?.unwrap_or(0);
    it.check_rows(l.len().saturating_mul(n))?;
    let mut out = Vec::with_capacity(l.len() * n);
    for _ in 0..n {
        out.extend_from_slice(l.items());
    }
    Ok(Value::List(ListValue::new(out)))
}

fn generate(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let initial = req_func(args, 0)?;
    let condition = req_func(args, 1)?;
    let next = req_func(args, 2)?;
    let selector = func_arg(args, 3)?;
    let mut out = Vec::new();
    let mut state = it.call(initial, &[])?;
    while truthy(it, condition, state.clone())? {
        it.check_rows(out.len() + 1)?;
        out.push(match selector {
            Some(s) => it.call(s, &[state.clone()])?,
            None => state.clone(),
        });
        state = it.call(next, &[state])?;
    }
    Ok(list_of(out))
}

fn difference(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut a = items(it, req_list(args, 0)?)?;
    let ic = is_ignore_case(arg(args, 2));
    for v in items(it, req_list(args, 1)?)? {
        if let Some(i) = index_of(it, &a, &v, ic)? {
            a.remove(i);
        }
    }
    Ok(list_of(a))
}

fn union(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let ic = is_ignore_case(arg(args, 1));
    let mut all = Vec::new();
    for l in items(it, req_list(args, 0)?)? {
        match l {
            Value::List(l) => all.extend(items(it, &l)?),
            other => return Err(convert_error(&other, "List")),
        }
    }
    Ok(list_of(dedup(it, all, ic)?))
}

fn intersect(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let ic = is_ignore_case(arg(args, 1));
    let lists = items(it, req_list(args, 0)?)?;
    let mut iter = lists.into_iter();
    let Some(Value::List(first)) = iter.next() else {
        return Ok(list_of(Vec::new()));
    };
    let mut acc = dedup(it, items(it, &first)?, ic)?;
    for l in iter {
        let Value::List(l) = l else {
            return Err(convert_error(&l, "List"));
        };
        let other = items(it, &l)?;
        let mut kept = Vec::new();
        for v in acc {
            if index_of(it, &other, &v, ic)?.is_some() {
                kept.push(v);
            }
        }
        acc = kept;
    }
    Ok(list_of(acc))
}

fn remove_items(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let remove = items(it, req_list(args, 1)?)?;
    let mut out = Vec::new();
    for v in items(it, req_list(args, 0)?)? {
        if index_of(it, &remove, &v, false)?.is_none() {
            out.push(v);
        }
    }
    Ok(list_of(out))
}

fn single(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    match l.items() {
        [one] => one.force(it),
        _ => Err(MError::expression(
            "There were too many elements in the enumeration to complete the operation.",
        )),
    }
}

fn single_or_default(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    match l.items() {
        [] => Ok(arg(args, 1).clone()),
        [one] => one.force(it),
        _ => Err(MError::expression(
            "There were too many elements in the enumeration to complete the operation.",
        )),
    }
}
