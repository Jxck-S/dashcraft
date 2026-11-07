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
    b.func("List.Dates", &["start", "count", "step"], dates);
    b.func("List.DateTimes", &["start", "count", "step"], datetimes);
    b.func(
        "List.DateTimeZones",
        &["start", "count", "step"],
        datetimezones,
    );
    b.func("List.Times", &["start", "count", "step"], times);
    b.func("List.Durations", &["start", "count", "step"], durations);
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
    b.func("List.Median", &["list", "?comparisonCriteria"], median);
    b.func("List.Mode", &["list", "?equationCriteria"], mode);
    b.func("List.Modes", &["list", "?equationCriteria"], modes);
    b.func(
        "List.StandardDeviation",
        &["numbersList"],
        standard_deviation,
    );
    b.func(
        "List.Covariance",
        &["numberList1", "numberList2"],
        covariance,
    );
    b.func("List.Product", &["numbersList", "?precision"], product);
    b.func(
        "List.Percentile",
        &["list", "percentiles", "?options"],
        percentile,
    );
    b.func("List.Split", &["list", "pageSize"], split);
    b.func(
        "List.Alternate",
        &["list", "count", "?repeatInterval", "?offset"],
        alternate,
    );
    b.func(
        "List.InsertRange",
        &["list", "index", "values"],
        insert_range,
    );
    b.func(
        "List.ReplaceRange",
        &["list", "index", "count", "replaceWith"],
        replace_range,
    );
    b.func(
        "List.RemoveRange",
        &["list", "index", "?count"],
        remove_range,
    );
    b.func(
        "List.ReplaceValue",
        &["list", "oldValue", "newValue", "replacer"],
        replace_value,
    );
    b.func("List.Positions", &["list"], positions);
    b.func(
        "List.IsDistinct",
        &["list", "?equationCriteria"],
        is_distinct,
    );
    b.func(
        "List.MaxN",
        &[
            "list",
            "countOrCondition",
            "?comparisonCriteria",
            "?includeNulls",
        ],
        max_n,
    );
    b.func(
        "List.MinN",
        &[
            "list",
            "countOrCondition",
            "?comparisonCriteria",
            "?includeNulls",
        ],
        min_n,
    );
    b.func(
        "List.RemoveLastN",
        &["list", "?countOrCondition"],
        remove_last_n,
    );
    b.func(
        "List.RemoveMatchingItems",
        &["list1", "list2", "?equationCriteria"],
        remove_matching_items,
    );
    b.func(
        "List.ReplaceMatchingItems",
        &["list", "replacements", "?equationCriteria"],
        replace_matching_items,
    );
    b.func(
        "List.TransformMany",
        &["list", "collectionTransform", "resultTransform"],
        transform_many,
    );
    b.func(
        "List.PositionOfAny",
        &["list", "values", "?occurrence", "?equationCriteria"],
        position_of_any,
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

/// `List.Dates` & co.: `count` values from `start`, each `step` (a
/// duration) after the previous one.
fn series(it: &Interp, args: &[Value], kind: &str) -> Result<Value, MError> {
    let start = arg(args, 0).plain().clone();
    let type_ok = match kind {
        "Date" => matches!(start, Value::Date(_)),
        "DateTime" => matches!(start, Value::DateTime(_)),
        "DateTimeZone" => matches!(start, Value::DateTimeZone(_)),
        "Time" => matches!(start, Value::Time(_)),
        _ => matches!(start, Value::Duration(_)),
    };
    if !type_ok {
        return Err(convert_error(&start, kind));
    }
    let step = arg(args, 2).plain().clone();
    if !matches!(step, Value::Duration(_)) {
        return Err(convert_error(&step, "Duration"));
    }
    let n = count_arg(args, 1)?.unwrap_or(0);
    it.check_rows(n)?;
    let mut out = Vec::with_capacity(n);
    let mut current = start;
    for i in 0..n {
        if i > 0 {
            current = arithmetic(BinaryOp::Add, &current, &step)?;
        }
        out.push(current.clone());
    }
    Ok(list_of(out))
}

fn dates(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    series(it, args, "Date")
}

fn datetimes(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    series(it, args, "DateTime")
}

fn datetimezones(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    series(it, args, "DateTimeZone")
}

fn times(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    series(it, args, "Time")
}

fn durations(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    series(it, args, "Duration")
}

fn reverse(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut v = req_list(args, 0)?.items().to_vec();
    v.reverse();
    Ok(Value::List(ListValue::new(v)))
}

fn sort(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let values = items(it, req_list(args, 0)?)?;
    Ok(list_of(sort_values(it, values, arg(args, 1))?))
}

/// Sorts `values` by a comparison criteria: `null`, an `Order` number, a key function, a
/// two-argument comparer or `{key, order}`.
fn sort_values(
    it: &Interp,
    mut values: Vec<Value>,
    criteria: &Value,
) -> Result<Vec<Value>, MError> {
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
            return Ok(values);
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
    Ok(keyed.into_iter().map(|(_, v)| v).collect())
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

fn not_enough_elements() -> MError {
    MError::expression(
        "There weren't enough elements in the enumeration to complete the operation.",
    )
}

/// The distinct values of `values` (in first-appearance order) with their counts.
fn value_counts(
    it: &Interp,
    values: Vec<Value>,
    ignore_case: bool,
) -> Result<Vec<(Value, usize)>, MError> {
    let mut buckets: HashMap<u64, Vec<usize>> = HashMap::new();
    let mut out: Vec<(Value, usize)> = Vec::new();
    for v in values {
        it.tick()?;
        let mut h = DefaultHasher::new();
        hash_value(&v, ignore_case, &mut h);
        let bucket = buckets.entry(h.finish()).or_default();
        let mut found = None;
        for &i in bucket.iter() {
            if equals_with(it, &out[i].0, &v, ignore_case)? {
                found = Some(i);
                break;
            }
        }
        match found {
            Some(i) => out[i].1 += 1,
            None => {
                bucket.push(out.len());
                out.push((v, 1));
            }
        }
    }
    Ok(out)
}

fn non_null_items(it: &Interp, args: &[Value]) -> Result<Vec<Value>, MError> {
    Ok(items(it, req_list(args, 0)?)?
        .into_iter()
        .filter(|v| !v.is_null())
        .collect())
}

fn numbers_of(values: Vec<Value>) -> Result<Vec<f64>, MError> {
    values
        .into_iter()
        .filter(|v| !v.is_null())
        .map(|v| v.as_number().ok_or_else(|| convert_error(&v, "Number")))
        .collect()
}

fn median(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let values = sort_values(it, non_null_items(it, args)?, arg(args, 1))?;
    let n = values.len();
    if n == 0 {
        return Ok(Value::Null);
    }
    let lower = &values[(n - 1) / 2];
    if n % 2 == 1 {
        return Ok(lower.clone());
    }
    match (lower, &values[n / 2]) {
        (Value::Number(a), Value::Number(b)) => Ok(Value::Number((a + b) / 2.0)),
        _ => Ok(lower.clone()),
    }
}

fn mode_counts(it: &Interp, args: &[Value]) -> Result<(Vec<(Value, usize)>, usize), MError> {
    let counts = value_counts(
        it,
        items(it, req_list(args, 0)?)?,
        is_ignore_case(arg(args, 1)),
    )?;
    let max = counts.iter().map(|(_, c)| *c).max().unwrap_or(0);
    Ok((counts, max))
}

fn mode(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let (counts, max) = mode_counts(it, args)?;
    // Among the most frequent values, the one that first appears last.
    Ok(counts
        .into_iter()
        .rev()
        .find(|(_, c)| *c == max)
        .map_or(Value::Null, |(v, _)| v))
}

fn modes(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let (counts, max) = mode_counts(it, args)?;
    Ok(list_of(
        counts
            .into_iter()
            .filter(|(_, c)| *c == max)
            .map(|(v, _)| v)
            .collect(),
    ))
}

fn standard_deviation(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let xs = numbers_of(items(it, req_list(args, 0)?)?)?;
    if xs.is_empty() {
        return Ok(Value::Null);
    }
    // Welford's running variance.
    let (mut mean, mut m2) = (0.0, 0.0);
    for (i, x) in xs.iter().enumerate() {
        let delta = x - mean;
        mean += delta / (i + 1) as f64;
        m2 += delta * (x - mean);
    }
    Ok(Value::Number(if xs.len() == 1 {
        0.0
    } else {
        (m2 / (xs.len() - 1) as f64).sqrt()
    }))
}

fn covariance(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let xs = numbers_of(items(it, req_list(args, 0)?)?)?;
    let ys = numbers_of(items(it, req_list(args, 1)?)?)?;
    if xs.len() != ys.len() {
        return Err(MError::expression(
            "The lists must have the same number of items.",
        ));
    }
    if xs.is_empty() {
        return Ok(Value::Null);
    }
    let (mut mx, mut my, mut mxy) = (0.0, 0.0, 0.0);
    for (i, (x, y)) in xs.iter().zip(&ys).enumerate() {
        let n = (i + 1) as f64;
        mx += (x - mx) / n;
        my += (y - my) / n;
        mxy += (x * y - mxy) / n;
    }
    Ok(Value::Number(mxy - mx * my))
}

fn product(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let xs = numbers_of(items(it, req_list(args, 0)?)?)?;
    Ok(xs
        .into_iter()
        .reduce(|a, b| a * b)
        .map_or(Value::Null, Value::Number))
}

/// The type name of a `List.Percentile` value, or `None` if it can't be interpolated.
fn percentile_kind(v: &Value) -> Option<&'static str> {
    match v {
        Value::Number(_) => Some("number"),
        Value::Date(_) => Some("date"),
        Value::DateTime(_) => Some("datetime"),
        Value::DateTimeZone(_) => Some("datetimezone"),
        Value::Duration(_) => Some("duration"),
        Value::Time(_) => Some("time"),
        _ => None,
    }
}

/// `lo + (hi - lo) * frac` in the values' own type.
fn interpolate(lo: &Value, hi: &Value, frac: f64) -> Result<Value, MError> {
    if frac == 0.0 {
        return Ok(lo.clone());
    }
    if let (Value::Number(a), Value::Number(b)) = (lo, hi) {
        return Ok(Value::Number(a + frac * (b - a)));
    }
    let delta = arithmetic(BinaryOp::Subtract, hi, lo)?;
    let step = arithmetic(BinaryOp::Multiply, &delta, &Value::Number(frac))?;
    arithmetic(BinaryOp::Add, lo, &step)
}

fn percentile(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let values = non_null_items(it, args)?;
    if let Some(first) = values.first() {
        if values
            .iter()
            .any(|v| std::mem::discriminant(v) != std::mem::discriminant(first))
        {
            return Err(MError::expression(
                "List.Percentile requires a list where all the values have the same type or are null.",
            ));
        }
        if percentile_kind(first).is_none() {
            return Err(MError::expression(
                "List.Percentile requires a list of dates, datetimes, datetimezones, durations, numbers or times.",
            ));
        }
    }
    let options = super::args::record_arg(args, 2)?;
    let mode = match super::args::option(it, options, "PercentileMode")? {
        None => 1.0,
        Some(v) => v.as_number().ok_or_else(|| convert_error(&v, "Number"))?,
    };
    let sorted = sort_values(it, values, &Value::Null)?;
    let one = |p: &Value| -> Result<Value, MError> {
        let p = p.as_number().ok_or_else(|| convert_error(p, "Number"))?;
        if !(0.0..=1.0).contains(&p) {
            return Err(MError::expression(
                "Percentile values must be between 0.0 and 1.0, inclusive.",
            ));
        }
        let n = sorted.len();
        if n == 0 {
            return Ok(Value::Null);
        }
        let at = |rank: f64, offset: usize| -> Result<Value, MError> {
            let lo = rank.floor() as usize;
            let frac = rank - rank.floor();
            let (Some(a), Some(b)) = (
                sorted.get(lo - offset),
                sorted.get((lo + 1 - offset).min(n - 1)),
            ) else {
                return Ok(Value::Null);
            };
            interpolate(a, b, frac)
        };
        match mode {
            2.0 => {
                let rank = (n + 1) as f64 * p;
                if rank < 1.0 || rank > n as f64 {
                    Ok(Value::Null)
                } else {
                    at(rank, 1)
                }
            }
            3.0 => {
                let i = ((p * n as f64).ceil() as usize)
                    .saturating_sub(1)
                    .min(n - 1);
                Ok(sorted[i].clone())
            }
            _ => at((n - 1) as f64 * p, 0),
        }
    };
    match arg(args, 1).plain() {
        Value::List(l) => Ok(list_of(
            items(it, l)?.iter().map(one).collect::<Result<_, _>>()?,
        )),
        p => one(p),
    }
}

fn split(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let size = match super::args::int_arg(args, 1)? {
        Some(n) if n > 0 => usize::try_from(n).unwrap_or(usize::MAX),
        _ => {
            return Err(MError::expression(
                "The 'pageSize' argument is out of range.",
            ));
        }
    };
    it.check_rows(l.len() / size + 1)?;
    Ok(list_of(
        l.items()
            .chunks(size)
            .map(|c| Value::List(ListValue::new(c.to_vec())))
            .collect(),
    ))
}

fn alternate(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let skip = count_arg(args, 1)?.unwrap_or(0);
    let take = count_arg(args, 2)?;
    let offset = count_arg(args, 3)?.unwrap_or(0);
    let keep = |i: usize| {
        if i < offset {
            return true;
        }
        let j = i - offset;
        match take {
            None => j >= skip,
            Some(take) => {
                let period = skip.saturating_add(take);
                period == 0 || j % period >= skip
            }
        }
    };
    Ok(Value::List(ListValue::new(
        l.items()
            .iter()
            .enumerate()
            .filter(|(i, _)| keep(*i))
            .map(|(_, v)| v.clone())
            .collect(),
    )))
}

fn splice(l: &ListValue, index: usize, remove: usize, insert: &[Lazy]) -> Result<Value, MError> {
    let items = l.items();
    let end = index.checked_add(remove).ok_or_else(not_enough_elements)?;
    if end > items.len() {
        return Err(not_enough_elements());
    }
    let mut out = Vec::with_capacity(items.len() - remove + insert.len());
    out.extend_from_slice(&items[..index]);
    out.extend_from_slice(insert);
    out.extend_from_slice(&items[end..]);
    Ok(Value::List(ListValue::new(out)))
}

fn insert_range(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let index = count_arg(args, 1)?.unwrap_or(0);
    let values = req_list(args, 2)?;
    it.check_rows(l.len().saturating_add(values.len()))?;
    splice(l, index, 0, values.items())
}

fn replace_range(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let index = count_arg(args, 1)?.unwrap_or(0);
    let count = count_arg(args, 2)?.unwrap_or(0);
    let values = req_list(args, 3)?;
    it.check_rows(l.len().saturating_add(values.len()))?;
    splice(l, index, count, values.items())
}

fn remove_range(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let index = count_arg(args, 1)?.unwrap_or(0);
    let count = count_arg(args, 2)?.unwrap_or(1);
    splice(l, index, count, &[])
}

fn replace_value(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let f = req_func(args, 3)?;
    let (old, new) = (arg(args, 1), arg(args, 2));
    let mut out = Vec::new();
    for v in items(it, req_list(args, 0)?)? {
        out.push(it.call(f, &[v, old.clone(), new.clone()])?);
    }
    Ok(list_of(out))
}

fn positions(_: &Interp, args: &[Value]) -> Result<Value, MError> {
    let n = req_list(args, 0)?.len();
    Ok(list_of((0..n).map(|i| Value::Number(i as f64)).collect()))
}

fn is_distinct(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let values = items(it, req_list(args, 0)?)?;
    let n = values.len();
    Ok(Value::Logical(
        dedup(it, values, is_ignore_case(arg(args, 1)))?.len() == n,
    ))
}

/// `List.MaxN` / `List.MinN`: the list sorted (largest first for `MaxN`), then the first
/// `count` items or the leading items matching the condition.
fn extreme_n(it: &Interp, args: &[Value], largest: bool) -> Result<Value, MError> {
    let include_nulls = super::args::bool_arg(args, 3)?.unwrap_or(false);
    let values: Vec<Value> = items(it, req_list(args, 0)?)?
        .into_iter()
        .filter(|v| include_nulls || !v.is_null())
        .collect();
    let mut sorted = sort_values(it, values, arg(args, 2))?;
    if largest {
        sorted.reverse();
    }
    let sorted = ListValue::from_values(sorted);
    let n = leading(it, &sorted, arg(args, 1), 0, false)?.min(sorted.len());
    Ok(slice(&sorted, 0, n))
}

fn max_n(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    extreme_n(it, args, true)
}

fn min_n(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    extreme_n(it, args, false)
}

fn remove_last_n(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let l = req_list(args, 0)?;
    let n = leading(it, l, arg(args, 1), 1, true)?.min(l.len());
    Ok(slice(l, 0, l.len() - n))
}

fn remove_matching_items(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let remove = items(it, req_list(args, 1)?)?;
    let ic = is_ignore_case(arg(args, 2));
    let mut out = Vec::new();
    for v in items(it, req_list(args, 0)?)? {
        if index_of(it, &remove, &v, ic)?.is_none() {
            out.push(v);
        }
    }
    Ok(list_of(out))
}

fn replace_matching_items(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let mut pairs = Vec::new();
    for r in items(it, req_list(args, 1)?)? {
        let Value::List(pair) = &r else {
            return Err(convert_error(&r, "List"));
        };
        let pair = items(it, pair)?;
        if pair.len() != 2 {
            return Err(MError::expression(
                "A replacement must be a list of an old and a new value.",
            ));
        }
        pairs.push((pair[0].clone(), pair[1].clone()));
    }
    let ic = is_ignore_case(arg(args, 2));
    let mut out = Vec::new();
    'items: for v in items(it, req_list(args, 0)?)? {
        for (old, new) in &pairs {
            if equals_with(it, &v, old, ic)? {
                out.push(new.clone());
                continue 'items;
            }
        }
        out.push(v);
    }
    Ok(list_of(out))
}

fn transform_many(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let collection = req_func(args, 1)?;
    let result = req_func(args, 2)?;
    let mut out = Vec::new();
    for x in items(it, req_list(args, 0)?)? {
        let ys = it.call(collection, std::slice::from_ref(&x))?;
        let Value::List(ys) = ys.plain() else {
            return Err(convert_error(&ys, "List"));
        };
        for y in items(it, ys)? {
            out.push(it.call(result, &[x.clone(), y])?);
            it.check_rows(out.len())?;
        }
    }
    Ok(list_of(out))
}

fn position_of_any(it: &Interp, args: &[Value]) -> Result<Value, MError> {
    let list = items(it, req_list(args, 0)?)?;
    let wanted = items(it, req_list(args, 1)?)?;
    let ic = is_ignore_case(arg(args, 3));
    let mut positions = Vec::new();
    for (i, x) in list.iter().enumerate() {
        if index_of(it, &wanted, x, ic)?.is_some() {
            positions.push(i as f64);
        }
    }
    Ok(match num_arg(args, 2)? {
        Some(1.0) => Value::Number(positions.last().copied().unwrap_or(-1.0)),
        Some(2.0) => list_of(positions.into_iter().map(Value::Number).collect()),
        _ => Value::Number(positions.first().copied().unwrap_or(-1.0)),
    })
}
