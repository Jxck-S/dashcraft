//! Evaluator semantics: laziness, closures, `each`, null propagation, errors, `try`, `meta`,
//! types, item/field access, intrinsics, sections, the global environment, and robustness
//! (cycles, deep recursion, limits, cancellation).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use indexmap::IndexMap;
use pbi_m::{Engine, EvalOptions, LimitKind, Limits, MError, Value};

fn eval(src: &str) -> Value {
    match Engine::default().evaluate(src) {
        Ok(v) => v,
        Err(e) => panic!("{src}\n=> {e}"),
    }
}

fn eval_err(src: &str) -> MError {
    match Engine::default().evaluate(src) {
        Ok(v) => panic!("expected an error for {src}, got {v}"),
        Err(e) => e,
    }
}

fn n(x: f64) -> Value {
    Value::Number(x)
}

fn t(s: &str) -> Value {
    Value::text(s)
}

fn reason(e: &MError) -> &str {
    match e {
        MError::Evaluation { reason, .. } => reason,
        other => panic!("not an evaluation error: {other:?}"),
    }
}

#[test]
fn let_is_lazy_and_memoized() {
    assert_eq!(eval("let a = error \"never\", b = 2 in b"), n(2.0));
    assert_eq!(eval("let x = y + 1, y = 41 in x"), n(42.0));
    assert_eq!(eval("let f = (x) => x * k, k = 3 in f(5)"), n(15.0));
    // Nested lets shadow outer bindings.
    assert_eq!(eval("let a = 1 in let a = 2 in a"), n(2.0));
}

#[test]
fn cyclic_let_is_an_error() {
    let e = eval_err("let a = b, b = a in a");
    assert!(e.to_string().contains("cyclic"), "{e}");
    let e = eval_err("let a = a + 1 in a");
    assert!(e.is_catchable(), "{e:?}");
    assert_eq!(
        eval("let a = b, b = a in try a otherwise \"caught\""),
        t("caught")
    );
}

#[test]
fn records_are_lazy_and_see_their_fields() {
    assert_eq!(eval("[a = 1, b = a + 1][b]"), n(2.0));
    assert_eq!(eval("[a = error \"x\", b = 2][b]"), n(2.0));
    let v = eval("[a = error \"x\", b = 2]");
    let r = v.as_record().unwrap_or_else(|| panic!("record"));
    assert!(r.field("a").is_some_and(|f| f.is_err()));
    assert_eq!(r.field("b").and_then(Result::ok), Some(n(2.0)));
}

#[test]
fn lists_are_lazy() {
    assert_eq!(eval("{error \"x\", 7}{1}"), n(7.0));
    assert_eq!(eval("List.Count({error \"x\", 7})"), n(2.0));
    assert_eq!(
        eval("{1..3, 5}"),
        Value::list([n(1.0), n(2.0), n(3.0), n(5.0)])
    );
    assert_eq!(eval("{3..1}"), Value::list([]));
}

#[test]
fn closures_each_and_recursion() {
    assert_eq!(eval("let add = (x) => (y) => x + y in add(2)(3)"), n(5.0));
    assert_eq!(eval("(each _ + 1)(2)"), n(3.0));
    assert_eq!(eval("(each [a] * 2)([a = 21])"), n(42.0));
    assert_eq!(
        eval("let fact = (n) => if n <= 1 then 1 else n * @fact(n - 1) in fact(10)"),
        n(3_628_800.0)
    );
    // Power Query also accepts recursion without `@` in a let.
    assert_eq!(
        eval("let fib = (n) => if n < 2 then n else fib(n - 1) + fib(n - 2) in fib(15)"),
        n(610.0)
    );
    assert_eq!(eval("let f = (a, optional b) => b ?? a in f(1)"), n(1.0));
    let e = eval_err("((a) => a)(1, 2)");
    assert!(e.to_string().contains("2 arguments"), "{e}");
    let e = eval_err("((a, b) => a)(1)");
    assert!(e.to_string().contains("expects 2"), "{e}");
}

#[test]
fn parameter_and_return_types_are_checked() {
    assert_eq!(eval("((x as text) => x)(\"a\")"), t("a"));
    let e = eval_err("((x as text) => x)(1)");
    assert!(e.to_string().contains("to type Text"), "{e}");
    assert_eq!(eval("((x as nullable text) => x)(null)"), Value::Null);
    assert!(
        Engine::default()
            .evaluate("((x as text) => x)(null)")
            .is_err()
    );
    let e = eval_err("((x) as number => \"no\")(1)");
    assert!(e.to_string().contains("Number"), "{e}");
}

#[test]
fn null_semantics() {
    assert_eq!(eval("null = null"), Value::Logical(true));
    assert_eq!(eval("null <> 1"), Value::Logical(true));
    assert_eq!(eval("null < 1"), Value::Null);
    assert_eq!(eval("1 + null"), Value::Null);
    assert_eq!(eval("\"a\" & null"), Value::Null);
    assert_eq!(eval("-null"), Value::Null);
    assert_eq!(eval("not null"), Value::Null);
    assert_eq!(eval("null ?? 5"), n(5.0));
    assert_eq!(eval("3 ?? (error \"rhs not evaluated\")"), n(3.0));
    assert_eq!(eval("null and false"), Value::Logical(false));
    assert_eq!(eval("null and true"), Value::Null);
    assert_eq!(eval("null or true"), Value::Logical(true));
    assert_eq!(eval("null or false"), Value::Null);
    assert_eq!(eval("false and (error \"x\")"), Value::Logical(false));
    assert_eq!(eval("true or (error \"x\")"), Value::Logical(true));
}

#[test]
fn operators() {
    assert_eq!(eval("1 = 1.0"), Value::Logical(true));
    assert_eq!(eval("1 = \"1\""), Value::Logical(false));
    assert_eq!(eval("\"a\" < \"b\""), Value::Logical(true));
    assert_eq!(eval("\"B\" < \"a\""), Value::Logical(true));
    assert_eq!(eval("{1, 2} & {3}"), Value::list([n(1.0), n(2.0), n(3.0)]));
    assert_eq!(eval("([a = 1] & [a = 2, b = 3])[a]"), n(2.0));
    assert_eq!(
        eval("[a = 1, b = 2] = [b = 2, a = 1]"),
        Value::Logical(true)
    );
    assert_eq!(eval("{1, {2}} = {1, {2}}"), Value::Logical(true));
    assert_eq!(eval("#nan = #nan"), Value::Logical(false));
    assert_eq!(eval("1 / 0"), n(f64::INFINITY));
    assert_eq!(
        eval("#date(2024, 1, 31) + #duration(1, 0, 0, 0)"),
        eval("#date(2024, 2, 1)")
    );
    assert_eq!(
        eval("#date(2024, 3, 1) - #date(2024, 2, 1)"),
        eval("#duration(29, 0, 0, 0)")
    );
    assert_eq!(
        eval("#date(2024, 1, 2) & #time(3, 4, 5)"),
        eval("#datetime(2024, 1, 2, 3, 4, 5)")
    );
    assert_eq!(
        eval("#datetimezone(2024, 1, 1, 1, 0, 0, 1, 0) = #datetimezone(2024, 1, 1, 0, 0, 0, 0, 0)"),
        Value::Logical(true)
    );
    let e = eval_err("1 + \"a\"");
    assert!(e.to_string().contains("operator +"), "{e}");
    let e = eval_err("{1} < {2}");
    assert!(e.to_string().contains("operator <"), "{e}");
    assert!(Engine::default().evaluate("if 1 then 2 else 3").is_err());
}

#[test]
fn errors_and_try() {
    let e = eval_err("error \"boom\"");
    assert_eq!(reason(&e), "Expression.Error");
    assert!(e.to_string().contains("boom"));
    let loc = e.location().unwrap_or_else(|| panic!("location"));
    assert_eq!((loc.line, loc.column), (1, 1));

    let e = eval_err("error [Reason = \"Custom.Error\", Message = \"m\", Detail = 42]");
    assert_eq!(reason(&e), "Custom.Error");
    let e = eval_err("error Error.Record(\"R\", \"msg\")");
    assert_eq!(reason(&e), "R");

    assert_eq!(eval("try error \"x\" otherwise 1"), n(1.0));
    assert_eq!(eval("try 5 otherwise 1"), n(5.0));
    assert_eq!(eval("(try error \"x\")[HasError]"), Value::Logical(true));
    assert_eq!(eval("(try error \"x\")[Error][Message]"), t("x"));
    assert_eq!(eval("(try 3)[Value]"), n(3.0));
    assert_eq!(
        eval("try error \"m\" catch (e) => e[Message] & \"!\""),
        t("m!")
    );
    assert_eq!(eval("try error \"m\" catch () => 0"), n(0.0));
    assert_eq!(eval("try ... otherwise \"ni\""), t("ni"));
    // Lazy: a record holding an error is not itself an error.
    assert_eq!(
        eval("(try [a = error \"x\"])[HasError]"),
        Value::Logical(false)
    );
}

#[test]
fn try_does_not_hide_unsupported_functions() {
    let e = eval_err("try Web.Contents(\"https://example.invalid\") otherwise 0");
    match &e {
        MError::Unsupported { function, hint, .. } => {
            assert_eq!(function, "Web.Contents");
            assert!(hint.contains("Load CSV manually"), "{hint}");
        }
        other => panic!("{other:?}"),
    }
    let e = eval_err("Table.NotAFunction(1)");
    assert!(matches!(&e, MError::Unsupported { function, .. } if function == "Table.NotAFunction"));
    let e = eval_err("File.Contents(\"C:\\x.csv\")");
    assert!(e.to_string().contains("never accesses files"), "{e}");
    // Unreferenced steps are never evaluated.
    assert_eq!(eval("let a = Web.Contents(\"x\"), b = 1 in b"), n(1.0));
    let e = eval_err("undefinedName + 1");
    assert!(e.to_string().contains("wasn't recognized"), "{e}");
}

#[test]
fn metadata_is_attached_and_ignored() {
    assert_eq!(eval("(\"v\" meta [k = 1]) & \"w\""), t("vw"));
    assert_eq!(eval("Value.Metadata(\"v\" meta [k = 1])[k]"), n(1.0));
    assert_eq!(
        eval("let p = 5 meta [IsParameterQuery = true] in p * 2"),
        n(10.0)
    );
    assert_eq!(
        eval("let p = 5 meta [IsParameterQuery = true] in Value.Metadata(p)[IsParameterQuery]"),
        Value::Logical(true)
    );
    assert_eq!(
        eval("Value.Metadata(Value.RemoveMetadata(1 meta [a = 1]))"),
        eval("[]")
    );
    assert_eq!(
        eval("Value.Metadata(1 meta [a = 1] meta [b = 2])[a]"),
        n(1.0)
    );
    assert_eq!(eval("(1 meta [a = 1]) = 1"), Value::Logical(true));
}

#[test]
fn as_is_and_types() {
    assert_eq!(eval("1 is number"), Value::Logical(true));
    assert_eq!(eval("null is nullable text"), Value::Logical(true));
    assert_eq!(eval("null is text"), Value::Logical(false));
    assert_eq!(eval("null is any"), Value::Logical(true));
    assert_eq!(eval("\"a\" as nullable text"), t("a"));
    assert!(Engine::default().evaluate("1 as text").is_err());
    assert_eq!(eval("Value.Is(1.5, Int64.Type)"), Value::Logical(true));
    assert_eq!(eval("Value.Type(\"a\") = type text"), Value::Logical(true));
    assert_eq!(
        eval("Type.IsNullable(type nullable number)"),
        Value::Logical(true)
    );
    assert_eq!(
        eval("Type.TableColumn(type table [A = text, B = Int64.Type], \"B\") = Int64.Type"),
        Value::Logical(true)
    );
    assert_eq!(
        eval("Value.Type(#table(type table [A = text], {})) = type table [A = text]"),
        Value::Logical(true)
    );
}

#[test]
fn field_and_item_access() {
    assert_eq!(eval("[a = 1][b]?"), Value::Null);
    let e = eval_err("[a = 1][b]");
    assert!(e.to_string().contains("'b'"), "{e}");
    assert_eq!(
        eval("[a = 1, b = 2, c = 3][[a], [c]]"),
        eval("[a = 1, c = 3]")
    );
    assert_eq!(eval("[a = 1][[a], [z]]?"), eval("[a = 1, z = null]"));
    assert_eq!(eval("{1, 2, 3}{2}"), n(3.0));
    assert_eq!(eval("{1}{5}?"), Value::Null);
    assert!(eval_err("{1}{5}").to_string().contains("enough elements"));

    let tbl = "#table({\"Name\", \"V\"}, {{\"a\", 1}, {\"b\", 2}, {\"b\", 3}})";
    assert_eq!(eval(&format!("{tbl}{{[Name = \"a\"]}}[V]")), n(1.0));
    assert_eq!(eval(&format!("{tbl}{{1}}[V]")), n(2.0));
    assert_eq!(
        eval(&format!("{tbl}[V]")),
        Value::list([n(1.0), n(2.0), n(3.0)])
    );
    assert_eq!(eval(&format!("{tbl}{{[Name = \"z\"]}}?")), Value::Null);
    assert!(
        eval_err(&format!("{tbl}{{[Name = \"z\"]}}"))
            .to_string()
            .contains("didn't match")
    );
    assert!(
        eval_err(&format!("{tbl}{{[Name = \"b\"]}}"))
            .to_string()
            .contains("more than one")
    );
    assert_eq!(
        eval(&format!("Table.ColumnNames({tbl}[[V]])")),
        Value::list([t("V")])
    );
}

#[test]
fn intrinsics() {
    let v = eval("#table(type table [A = text, B = number], {{\"x\", 1}})");
    let table = v.as_table().unwrap_or_else(|| panic!("table"));
    assert_eq!(table.row_count(), 1);
    assert_eq!(
        table.column_type(1).map(|t| t.to_string()),
        Some("number".into())
    );
    assert_eq!(
        eval("Table.ColumnNames(#table(2, {}))"),
        Value::list([t("Column1"), t("Column2")])
    );
    assert!(
        Engine::default()
            .evaluate("#table({\"a\"}, {{1, 2}})")
            .is_err()
    );
    assert_eq!(eval("Date.Year(#date(2020, 1, 31))"), n(2020.0));
    assert!(Engine::default().evaluate("#date(2020, 2, 30)").is_err());
    assert_eq!(eval("Time.Hour(#time(13, 0, 0))"), n(13.0));
    assert_eq!(eval("Duration.TotalHours(#duration(1, 2, 0, 0))"), n(26.0));
    assert_eq!(
        eval("DateTimeZone.ZoneHours(#datetimezone(2020, 1, 1, 0, 0, 0, -5, 0))"),
        n(-5.0)
    );
    assert_eq!(eval("#infinity > 1e308"), Value::Logical(true));
    assert_eq!(eval("Binary.Length(#binary({1, 2, 3}))"), n(3.0));
    assert_eq!(eval("Text.FromBinary(#binary(\"aGk=\"))"), t("hi"));
    assert!(
        eval("#shared")
            .as_record()
            .is_some_and(|r| r.get("Text.Trim").is_some())
    );
}

#[test]
fn shared_queries_and_parameters() {
    let mut shared = IndexMap::new();
    shared.insert(
        "Url".to_owned(),
        "\"https://x.invalid/c\" meta [IsParameterQuery = true]".to_owned(),
    );
    shared.insert("Twice".to_owned(), "(x) => x * 2".to_owned());
    shared.insert("Broken".to_owned(), "let in".to_owned());
    shared.insert("Loop".to_owned(), "Loop2".to_owned());
    shared.insert("Loop2".to_owned(), "Loop".to_owned());
    let engine = Engine::default();
    assert_eq!(
        engine
            .evaluate_query("Text.Upper(Url) & Text.From(Twice(21))", &shared)
            .ok(),
        Some(t("HTTPS://X.INVALID/C42"))
    );
    // A syntax error in an unreferenced shared query does not matter...
    assert!(engine.evaluate_query("1", &shared).is_ok());
    // ...but is reported, with the query name, when referenced.
    match engine.evaluate_query("Broken", &shared) {
        Err(MError::Syntax { query, line, .. }) => {
            assert_eq!(query.as_deref(), Some("Broken"));
            assert_eq!(line, 1);
        }
        other => panic!("{other:?}"),
    }
    let e = engine.evaluate_query("Loop", &shared).err();
    assert!(e.is_some_and(|e| e.to_string().contains("cyclic")));
    // Shared queries shadow library names.
    let mut shadow = IndexMap::new();
    shadow.insert("Text.Upper".to_owned(), "(x) => \"shadowed\"".to_owned());
    assert_eq!(
        engine.evaluate_query("Text.Upper(\"a\")", &shadow).ok(),
        Some(t("shadowed"))
    );
}

#[test]
fn syntax_errors_have_line_and_column() {
    match Engine::default().evaluate("let\n  a = \n in a") {
        Err(e @ MError::Syntax { .. }) => {
            let MError::Syntax {
                line,
                column,
                query,
                ..
            } = &e
            else {
                unreachable!()
            };
            assert_eq!(query, &None);
            assert_eq!((*line, *column), (3, 2));
            assert!(e.to_string().contains("line 3, column 2"), "{e}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn sections_evaluate_to_member_records() {
    let src = "section S; shared A = 1; B = A + 1; shared C = S!B * 10;";
    let v = eval(src);
    let r = v.as_record().unwrap_or_else(|| panic!("record"));
    assert_eq!(r.field("B").and_then(Result::ok), Some(n(2.0)));
    assert_eq!(r.field("C").and_then(Result::ok), Some(n(20.0)));
    // Members that use unsupported functions are reported per member.
    let hello = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/hello_world.pq"),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let v = eval(&hello);
    let r = v.as_record().unwrap_or_else(|| panic!("record"));
    assert!(matches!(
        r.field("HelloWorld.Contents"),
        Some(Ok(Value::Function(_)))
    ));
    assert!(matches!(r.field("HelloWorld"), Some(Ok(Value::Record(_)))));
}

#[test]
fn deep_recursion_is_an_error_not_a_stack_overflow() {
    let e = eval_err("let f = (n) => if n = 0 then 0 else 1 + @f(n - 1) in f(1000000)");
    assert!(
        matches!(
            e,
            MError::LimitExceeded {
                kind: LimitKind::Depth,
                ..
            }
        ),
        "{e:?}"
    );
    // Within the limit, recursion works.
    assert_eq!(
        eval("let f = (n) => if n = 0 then 0 else 1 + @f(n - 1) in f(300)"),
        n(300.0)
    );
    // Long binding chains are bounded too.
    let mut src = String::from("let a0 = 0");
    for i in 1..5000 {
        src.push_str(&format!(", a{i} = a{} + 1", i - 1));
    }
    src.push_str(" in a4999");
    let e = eval_err(&src);
    assert!(matches!(e, MError::LimitExceeded { .. }), "{e:?}");
}

#[test]
fn huge_lists_respect_limits() {
    let e = eval_err("List.Count({1..100000000})");
    assert!(
        matches!(
            e,
            MError::LimitExceeded {
                kind: LimitKind::Rows,
                ..
            }
        ),
        "{e:?}"
    );
    let e = eval_err("List.Accumulate({1..40}, {1}, (s, x) => s & s)");
    assert!(
        matches!(
            e,
            MError::LimitExceeded {
                kind: LimitKind::Rows,
                ..
            }
        ),
        "{e:?}"
    );
    let e = eval_err("List.Accumulate({1..40}, \"ab\", (s, x) => s & s)");
    assert!(
        matches!(
            e,
            MError::LimitExceeded {
                kind: LimitKind::Bytes,
                ..
            }
        ),
        "{e:?}"
    );
    let small = Engine::new(EvalOptions {
        limits: Limits {
            max_rows: 10,
            ..Limits::default()
        },
        ..EvalOptions::default()
    });
    assert!(small.evaluate("{1..10}").is_ok());
    assert!(matches!(
        small.evaluate("Table.FromRows(List.Transform({1..10}, each {_}) & {{0}})"),
        Err(MError::LimitExceeded { .. })
    ));
    let steps = Engine::new(EvalOptions {
        limits: Limits {
            max_steps: 10_000,
            ..Limits::default()
        },
        ..EvalOptions::default()
    });
    assert!(matches!(
        steps.evaluate("List.Sum(List.Transform({1..100000}, each _ * 2))"),
        Err(MError::LimitExceeded {
            kind: LimitKind::Steps,
            ..
        })
    ));
}

#[test]
fn cancellation_stops_evaluation() {
    let cancel = Arc::new(AtomicBool::new(false));
    let engine = Engine::new(EvalOptions {
        cancel: cancel.clone(),
        ..EvalOptions::default()
    });
    let flag = cancel.clone();
    let setter = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        flag.store(true, Ordering::Relaxed);
    });
    let r = engine.evaluate("List.Accumulate({1..5000000}, 0, (s, x) => s + Number.Mod(x, 7) * 2)");
    let _ = setter.join();
    assert!(matches!(r, Err(MError::Cancelled)), "{r:?}");
    // A pre-set flag cancels immediately.
    assert!(matches!(engine.evaluate("1 + 1"), Err(MError::Cancelled)));
    cancel.store(false, Ordering::Relaxed);
    assert_eq!(engine.evaluate("1 + 1").ok(), Some(n(2.0)));
}

#[test]
fn display_messages_are_clear() {
    let e = eval_err("let x = 1 in\n  Text.Upper(x)");
    let msg = e.to_string();
    assert!(
        msg.starts_with("Expression.Error: We cannot convert the value 1 to type Text."),
        "{msg}"
    );
    assert!(msg.contains("line 2, column 3"), "{msg}");
}

#[test]
fn deeply_nested_values_drop_without_recursion() {
    // Nested lists, records and closures/environments 200k levels deep.
    assert_eq!(
        eval("List.Count(List.Accumulate({1..200000}, {}, (s, x) => {s}))"),
        n(1.0)
    );
    assert_eq!(
        eval("Record.FieldCount(List.Accumulate({1..200000}, [], (s, x) => [a = s]))"),
        n(1.0)
    );
    assert_eq!(
        eval("List.Count(List.Accumulate({1..200000}, {}, (s, x) => List.Transform({1}, each s)))"),
        n(1.0)
    );
    assert_eq!(
        eval("Value.Is(List.Accumulate({1..100000}, type text, (s, x) => type {s}), type type)"),
        Value::Logical(true)
    );
    // Resolving the deep value for the caller hits the depth limit instead of the stack.
    let e = eval_err("List.Accumulate({1..200000}, {}, (s, x) => {s})");
    assert!(
        matches!(
            e,
            MError::LimitExceeded {
                kind: LimitKind::Depth,
                ..
            }
        ),
        "{e:?}"
    );
}

#[test]
fn sample_documents_evaluate_lazily() {
    let read = |name: &str| {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/data")
                .join(name),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"))
    };
    // Only `Records` and `Source` are needed; the other bindings are never evaluated.
    assert_eq!(eval(&read("grammar_tour.m")), n(1.0));
    // `Final = try Grouped otherwise ...`: the function expression is evaluated before its
    // arguments, and `try` does not hide unsupported functions.
    let src = read("desktop_generated.m");
    let line = src
        .lines()
        .position(|l| l.contains("Table.Group("))
        .map(|i| i + 1);
    match Engine::default().evaluate(&src) {
        Err(MError::Unsupported {
            function, location, ..
        }) => {
            assert_eq!(function, "Table.Group");
            assert_eq!(location.map(|l| l.line), line);
        }
        other => panic!("{other:?}"),
    }
}
