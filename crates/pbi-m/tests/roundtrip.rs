//! Printer round-trip: `parse(print(parse(x))) == parse(x)` modulo spans.

use std::path::PathBuf;

use pbi_m::ast::Document;
use pbi_m::{parse_document, parse_expression, print_document, print_expr};
use pretty_assertions::assert_eq;

fn without_spans(mut doc: Document) -> Document {
    doc.clear_spans();
    doc
}

fn assert_round_trip(src: &str, origin: &str) {
    let parsed = parse_document(src).unwrap_or_else(|e| panic!("{origin}: {}", e.render(src)));
    let printed = print_document(&parsed);
    let reparsed = parse_document(&printed).unwrap_or_else(|e| {
        panic!(
            "{origin}: printed output does not parse: {}\n{printed}",
            e.render(&printed)
        )
    });
    assert_eq!(
        without_spans(reparsed.clone()),
        without_spans(parsed),
        "{origin}: printed:\n{printed}"
    );
    // Printing is a fixed point after one round.
    assert_eq!(print_document(&reparsed), printed, "{origin}");
}

#[test]
fn round_trip_all_test_data() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let src = std::fs::read_to_string(&path).unwrap();
        assert_round_trip(&src, &path.display().to_string());
    }
}

#[test]
fn round_trip_snippets() {
    let snippets = [
        "1 + 2 * 3",
        "(1 + 2) * 3",
        "a - (b - c)",
        "a ?? b ?? c",
        "a ?? (b ?? c)",
        "not (a and b)",
        "-(-1)",
        "- -1",
        "x as number is logical",
        "(x is number) as logical",
        "a = (b as number)",
        "\"x\" meta [a = 1] meta [b = 2]",
        "type nullable {text}",
        "type function (optional x as nullable text) as table [a = Int64.Type]",
        "type [optional #\"optional x\" = text, #\"optional y\", ...]",
        "type table [#\"optional z\" = number]",
        "(type text) meta [a = 1]",
        "try (try a) otherwise b",
        "try (each try a) otherwise b",
        "try (let x = try a in x) catch (e) => e",
        "try try a otherwise b",
        "try if a then b else try c otherwise d",
        "each if [a] then [b] else [c]",
        "(each _)(1)",
        "(x) as number",
        "((x) => x)(1)",
        "{1..10, 20, (each _)}",
        "[a = \"#(lf)\", b = \"tab#(tab)\", c = \"#(#)(\", d = \"\"\"\"]",
        "@f",
        "Section1!#\"Member Name\"",
        "#\"each\"[#\"a]\"]",
        "[Column1.2]? ?? 0",
        "{0}{0}?",
        "error \"a\"",
        "#!\"verbatim\"",
        "x{[Name = \"a\"]}[[a], [#\"b c\"]]?",
        "#date(2024, 1, 2)",
        "#infinity",
        "...",
    ];
    for src in snippets {
        assert_round_trip(src, src);
    }
}

#[test]
fn printer_inserts_parentheses_for_built_trees() {
    use pbi_m::Span;
    use pbi_m::ast::{BinaryOp, Expr, ExprKind, Literal, Name, NumberLiteral};
    let num = |v: f64| {
        Expr::new(
            ExprKind::Literal(Literal::Number(NumberLiteral::from_f64(v))),
            Span::default(),
        )
    };
    let bin = |op, l: Expr, r: Expr| {
        Expr::new(
            ExprKind::Binary {
                op,
                lhs: Box::new(l),
                rhs: Box::new(r),
            },
            Span::default(),
        )
    };
    // (1 + 2) * (3 - 4) built without Parenthesized nodes.
    let e = bin(
        BinaryOp::Multiply,
        bin(BinaryOp::Add, num(1.0), num(2.0)),
        bin(BinaryOp::Subtract, num(3.0), num(4.0)),
    );
    assert_eq!(print_expr(&e), "(1 + 2) * (3 - 4)");
    // 1 - (2 - 3)
    let e = bin(
        BinaryOp::Subtract,
        num(1.0),
        bin(BinaryOp::Subtract, num(2.0), num(3.0)),
    );
    assert_eq!(print_expr(&e), "1 - (2 - 3)");
    // An `each` operand needs parentheses.
    let each = Expr::new(ExprKind::Each(Box::new(num(1.0))), Span::default());
    let e = bin(BinaryOp::Add, each.clone(), num(2.0));
    assert_eq!(print_expr(&e), "(each 1) + 2");
    // Names that need quoting get quoted.
    let id = Expr::new(
        ExprKind::Identifier {
            name: Name::new("My Step"),
            inclusive: false,
        },
        Span::default(),
    );
    assert_eq!(print_expr(&id), "#\"My Step\"");
    let field = Expr::new(
        ExprKind::FieldAccess {
            target: Some(Box::new(id)),
            field: Name::new("Shipped (UTC)"),
            optional: false,
        },
        Span::default(),
    );
    assert_eq!(print_expr(&field), "#\"My Step\"[#\"Shipped (UTC)\"]");
    assert!(parse_expression(&print_expr(&field)).is_ok());
    // A handler-less `try` at the end of a protected expression would capture `otherwise`.
    let ident = |s: &str| {
        Expr::new(
            ExprKind::Identifier {
                name: Name::new(s),
                inclusive: false,
            },
            Span::default(),
        )
    };
    let open_try = Expr::new(
        ExprKind::Try {
            expr: Box::new(ident("a")),
            handler: None,
        },
        Span::default(),
    );
    let e = Expr::new(
        ExprKind::Try {
            expr: Box::new(Expr::new(
                ExprKind::Each(Box::new(open_try)),
                Span::default(),
            )),
            handler: Some(pbi_m::ast::TryHandler::Otherwise(Box::new(ident("b")))),
        },
        Span::default(),
    );
    assert_eq!(print_expr(&e), "try (each try a) otherwise b");
}

#[test]
fn let_layout() {
    let e = parse_expression("let a = 1, b = let c = 2 in c in a + b").unwrap();
    assert_eq!(
        print_expr(&e),
        "let\n    a = 1,\n    b = let\n        c = 2\n    in\n        c\nin\n    a + b"
    );
}
