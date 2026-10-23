#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Printer round-trip tests: `parse(print(parse(x))) == parse(x)` modulo
//! spans, over the test data, generated expressions and random token soup.

mod common;

use common::{data_files, sexp};
use pbi_dax::syntax::*;
use pretty_assertions::assert_eq;

const WIDTHS: [usize; 4] = [120, 80, 30, 1];

fn check_spans(src: &str, root: &Expr) {
    for_each_expr(root, |e| {
        assert!(
            e.span.start <= e.span.end && e.span.end <= src.len(),
            "bad span {:?} in {src:?}",
            e.span
        );
        if e.kind != ExprKind::Omitted {
            assert!(!e.span.is_empty(), "empty span for {e:?}");
        }
        for child in e.children() {
            assert!(
                e.span.start <= child.span.start && child.span.end <= e.span.end,
                "child span {:?} outside parent {:?} in {src:?}",
                child.span,
                e.span
            );
        }
        if let ExprKind::Binary { lhs, rhs, .. } = &e.kind {
            assert_eq!(e.span, lhs.span.to(rhs.span));
        }
    });
}

fn check_expr_roundtrip(src: &str, expr: &Expr) {
    check_spans(src, expr);
    let printed = print_expr(expr);
    let reparsed = parse_expression(&printed).unwrap_or_else(|e| {
        panic!(
            "re-parse failed: {}\nsource:  {src}\nprinted: {printed}",
            e.render(&printed)
        )
    });
    assert_eq!(
        sexp(&reparsed),
        sexp(expr),
        "\nsource:  {src}\nprinted: {printed}"
    );
    assert!(
        reparsed.eq_ignoring_spans(expr),
        "source: {src}\nprinted: {printed}"
    );
    assert_eq!(print_expr(&reparsed), printed, "printing is not idempotent");
    for max_width in WIDTHS {
        let opts = PrintOptions {
            max_width,
            ..PrintOptions::default()
        };
        let pretty = pretty_expr(expr, &opts);
        let reparsed = parse_expression(&pretty).unwrap_or_else(|e| {
            panic!(
                "re-parse of pretty output failed: {}\nsource: {src}\npretty:\n{pretty}",
                e.render(&pretty)
            )
        });
        assert!(
            reparsed.eq_ignoring_spans(expr),
            "source: {src}\npretty:\n{pretty}\n{}\n{}",
            sexp(&reparsed),
            sexp(expr)
        );
    }
}

fn check_query_roundtrip(src: &str, query: &Query) {
    for e in query.expressions() {
        check_spans(src, e);
    }
    let printed = print_query(query);
    let reparsed = parse_query(&printed).unwrap_or_else(|e| {
        panic!(
            "re-parse failed: {}\nsource:\n{src}\nprinted:\n{printed}",
            e.render(&printed)
        )
    });
    assert!(
        reparsed.eq_ignoring_spans(query),
        "source:\n{src}\nprinted:\n{printed}"
    );
    for max_width in WIDTHS {
        let opts = PrintOptions {
            max_width,
            ..PrintOptions::default()
        };
        let pretty = pretty_query(query, &opts);
        let reparsed = parse_query(&pretty).unwrap_or_else(|e| {
            panic!(
                "re-parse of pretty output failed: {}\nsource:\n{src}\npretty:\n{pretty}",
                e.render(&pretty)
            )
        });
        assert!(
            reparsed.eq_ignoring_spans(query),
            "source:\n{src}\npretty:\n{pretty}"
        );
    }
}

#[test]
fn roundtrip_all_test_data() {
    for (path, src) in data_files("").iter().chain(data_files("extra").iter()) {
        let expr = parse_expression(src)
            .unwrap_or_else(|e| panic!("{}: {}", path.display(), e.render(src)));
        check_expr_roundtrip(src, &expr);
    }
    for (path, src) in data_files("queries") {
        let query =
            parse_query(&src).unwrap_or_else(|e| panic!("{}: {}", path.display(), e.render(&src)));
        check_query_roundtrip(&src, &query);
    }
}

#[test]
fn canonical_and_pretty_output() {
    let (_, src) = data_files("")
        .into_iter()
        .find(|(p, _)| p.ends_with("table_products_never_sold.dax"))
        .unwrap();
    let expr = parse_expression(&src).unwrap();
    assert_eq!(
        print_expr(&expr),
        "VAR SoldProducts = SELECTCOLUMNS(SUMMARIZE(Orders, Orders[Product ID]), \"Product ID\", \
         Orders[Product ID]) VAR Catalog = SELECTCOLUMNS(ALL(Products), \"Product ID\", \
         Products[Product ID]) VAR NeverSold = EXCEPT(Catalog, SoldProducts) RETURN \
         SELECTCOLUMNS(FILTER(ALL(Products), Products[Product ID] IN NeverSold && \
         ISEMPTY(RELATEDTABLE(Orders))), \"Product ID\", Products[Product ID], \"Product\", \
         Products[Product Name], \"Category\", Products[Category], \"List Price\", \
         Products[List Price])"
    );
    assert_eq!(
        pretty_expr(&expr, &PrintOptions::default()),
        "VAR SoldProducts =
    SELECTCOLUMNS(
        SUMMARIZE(Orders, Orders[Product ID]),
        \"Product ID\",
        Orders[Product ID]
    )
VAR Catalog = SELECTCOLUMNS(ALL(Products), \"Product ID\", Products[Product ID])
VAR NeverSold = EXCEPT(Catalog, SoldProducts)
RETURN
    SELECTCOLUMNS(
        FILTER(
            ALL(Products),
            Products[Product ID] IN NeverSold && ISEMPTY(RELATEDTABLE(Orders))
        ),
        \"Product ID\",
        Products[Product ID],
        \"Product\",
        Products[Product Name],
        \"Category\",
        Products[Category],
        \"List Price\",
        Products[List Price]
    )"
    );

    let expr = parse_expression("VAR a = 1 VAR b = a + 1 RETURN IF(b > 1, \"x\", \"y\")").unwrap();
    assert_eq!(
        pretty_expr(&expr, &PrintOptions::default()),
        "VAR a = 1\nVAR b = a + 1\nRETURN\n    IF(b > 1, \"x\", \"y\")"
    );

    let q =
        parse_query("define measure T[m] = 1 evaluate T order by T[a] desc start at 1").unwrap();
    assert_eq!(
        print_query(&q),
        "DEFINE\n    MEASURE T[m] = 1\n\nEVALUATE\n    T\nORDER BY T[a] DESC\nSTART AT 1\n"
    );
    // Display uses the canonical printer.
    assert_eq!(
        parse_expression("calculate( [m] ,T[a]=1 )")
            .unwrap()
            .to_string(),
        "calculate([m], T[a] = 1)"
    );
}

// ---- programmatic trees: parentheses are inserted where required ---------------

fn n(i: i64) -> Expr {
    Expr::new(ExprKind::Number(Number::Integer(i)), Span::default())
}

fn col(name: &str) -> Expr {
    Expr::new(
        ExprKind::ColumnRef {
            table: None,
            column: name.into(),
        },
        Span::default(),
    )
}

fn bin(op: BinaryOp, lhs: Expr, rhs: Expr) -> Expr {
    Expr::new(
        ExprKind::Binary {
            op,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        Span::default(),
    )
}

fn un(op: UnaryOp, e: Expr) -> Expr {
    Expr::new(
        ExprKind::Unary {
            op,
            expr: Box::new(e),
        },
        Span::default(),
    )
}

fn var_block() -> Expr {
    Expr::new(
        ExprKind::Var {
            bindings: vec![VarBinding {
                name: "v".into(),
                name_span: Span::default(),
                expr: n(1),
            }],
            body: Box::new(Expr::new(ExprKind::VarRef("v".into()), Span::default())),
        },
        Span::default(),
    )
}

fn strip_parens(e: &Expr) -> Expr {
    let mut e = e.unparen().clone();
    for child in e.children_mut() {
        *child = strip_parens(child);
    }
    e.without_spans()
}

#[test]
fn printer_inserts_required_parentheses() {
    use BinaryOp::*;
    let cases = [
        (bin(Mul, bin(Add, n(1), n(2)), n(3)), "(1 + 2) * 3"),
        (bin(Sub, n(1), bin(Sub, n(2), n(3))), "1 - (2 - 3)"),
        (bin(Sub, bin(Sub, n(1), n(2)), n(3)), "1 - 2 - 3"),
        (bin(Pow, n(2), bin(Pow, n(3), n(2))), "2 ^ (3 ^ 2)"),
        (un(UnaryOp::Minus, bin(Add, n(1), n(2))), "-(1 + 2)"),
        (bin(Pow, un(UnaryOp::Minus, n(2)), n(2)), "(-2) ^ 2"),
        (un(UnaryOp::Minus, bin(Pow, n(2), n(2))), "-2 ^ 2"),
        (un(UnaryOp::Minus, un(UnaryOp::Minus, n(1))), "- -1"),
        (bin(Mul, un(UnaryOp::Minus, n(1)), n(2)), "-1 * 2"),
        (
            bin(And, un(UnaryOp::Not, col("x")), col("y")),
            "NOT [x] && [y]",
        ),
        (
            bin(Eq, bin(Mul, n(1), un(UnaryOp::Not, col("x"))), n(2)),
            "(1 * NOT [x]) = 2",
        ),
        (bin(Add, var_block(), n(1)), "(VAR v = 1 RETURN v) + 1"),
        (bin(Add, n(1), var_block()), "1 + VAR v = 1 RETURN v"),
        (
            bin(Concat, bin(Or, col("a"), col("b")), col("c")),
            "([a] || [b]) & [c]",
        ),
    ];
    for (tree, expected) in cases {
        let printed = print_expr(&tree);
        assert_eq!(printed, expected);
        let reparsed = parse_expression(&printed).unwrap();
        assert_eq!(sexp(&strip_parens(&reparsed)), sexp(&tree), "{printed}");
    }
    // `NOT (…)` re-parses as the equivalent function form.
    let tree = un(UnaryOp::Not, bin(And, col("x"), col("y")));
    assert_eq!(print_expr(&tree), "NOT ([x] && [y])");
}

#[test]
fn table_names_colliding_with_variables_stay_quoted() {
    let src = "VAR t = 1 RETURN 't' + t + COUNTROWS('Year') + COUNTROWS(Orders)";
    let expr = parse_expression(src).unwrap();
    assert_eq!(print_expr(&expr), src);
    check_expr_roundtrip(src, &expr);
}

// ---- generated input ----------------------------------------------------------

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

const ATOMS: &[&str] = &[
    "1",
    "0",
    "2.5",
    ".5",
    "1e3",
    "1.",
    "\"a\"",
    "\"q\"\"q\"",
    "\"\"",
    "TRUE",
    "FALSE()",
    "[m]",
    "[a]]b]",
    "T[c]",
    "'Web Sessions'[Traffic Source]",
    "'It''s'[x]",
    "Orders",
    "'Web Sessions'",
    "'Table'",
    "'v0'",
    "BLANK()",
    "DESC",
    "Year",
    "@p",
    "dt\"2020-01-01\"",
    "v0",
    "v1",
    "_x",
];

const BINOPS: &[&str] = &[
    "+", "-", "*", "/", "^", "&", "=", "==", "<>", "<", ">", "<=", ">=", "&&", "||", "IN", "in",
];

const FUNCS: &[&str] = &[
    "CALCULATE",
    "IF",
    "SumX",
    "PERCENTILE.INC",
    "NOT",
    "f",
    "TRUE",
];

fn gen_expr(rng: &mut Rng, depth: usize) -> String {
    if depth == 0 {
        return rng.pick(ATOMS).to_string();
    }
    let d = depth - 1;
    match rng.below(12) {
        0 | 1 => rng.pick(ATOMS).to_string(),
        2..=4 => format!(
            "{} {} {}",
            gen_expr(rng, d),
            rng.pick(BINOPS),
            gen_expr(rng, d)
        ),
        5 => format!("({})", gen_expr(rng, d)),
        6 => format!(
            "{} {}",
            rng.pick(&["-", "+", "NOT", "not"]),
            gen_expr(rng, d)
        ),
        7 => {
            let name = rng.pick(FUNCS);
            if name == "TRUE" {
                return "TRUE()".to_string();
            }
            let args: Vec<String> = (0..rng.below(4))
                .map(|_| {
                    if rng.below(8) == 0 {
                        String::new()
                    } else {
                        gen_expr(rng, d)
                    }
                })
                .collect();
            format!("{name}({})", args.join(", "))
        }
        8 => {
            let mut s = String::new();
            for i in 0..=rng.below(3) {
                s.push_str(&format!("VAR v{i} = {} ", gen_expr(rng, d)));
            }
            format!("{s}RETURN {}", gen_expr(rng, d))
        }
        9 => gen_table(rng, d),
        10 => format!("{} IN {}", gen_expr(rng, d), gen_table(rng, d)),
        _ => format!("({}, {})", gen_expr(rng, d), gen_expr(rng, d)),
    }
}

fn gen_table(rng: &mut Rng, depth: usize) -> String {
    let rows: Vec<String> = (0..rng.below(4))
        .map(|_| {
            if rng.below(2) == 0 {
                gen_expr(rng, depth)
            } else {
                format!("({}, {})", gen_expr(rng, depth), gen_expr(rng, depth))
            }
        })
        .collect();
    format!("{{{}}}", rows.join(", "))
}

fn gen_query(rng: &mut Rng) -> String {
    let mut s = String::new();
    if rng.below(2) == 0 {
        s.push_str("DEFINE\n");
        for _ in 0..=rng.below(4) {
            let def = match rng.below(5) {
                0 => format!("MEASURE 'T'[m] = {}", gen_expr(rng, 3)),
                1 => format!("VAR v0 = {}", gen_expr(rng, 3)),
                2 => format!("TABLE 'My Table' = {}", gen_expr(rng, 3)),
                3 => format!("COLUMN T[c] = {}", gen_expr(rng, 3)),
                _ => format!(
                    "FUNCTION F.G = (v1 : SCALAR VAL, x) => {}",
                    gen_expr(rng, 3)
                ),
            };
            s.push_str(&def);
            s.push('\n');
        }
    }
    for _ in 0..=rng.below(2) {
        s.push_str(&format!("EVALUATE {}\n", gen_expr(rng, 3)));
        if rng.below(2) == 0 {
            let keys: Vec<String> = (0..=rng.below(2))
                .map(|_| format!("{} {}", gen_expr(rng, 2), rng.pick(&["", "ASC", "DESC"])))
                .collect();
            s.push_str(&format!("ORDER BY {}\n", keys.join(", ")));
            if rng.below(2) == 0 {
                s.push_str(&format!(
                    "START AT {}, {}\n",
                    gen_expr(rng, 1),
                    gen_expr(rng, 1)
                ));
            }
        }
    }
    s
}

#[test]
fn generated_expressions_roundtrip() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _ in 0..3_000 {
        let depth = rng.below(6);
        let src = gen_expr(&mut rng, depth);
        let expr = parse_expression(&src)
            .unwrap_or_else(|e| panic!("generated input failed: {}\n{src}", e.render(&src)));
        check_expr_roundtrip(&src, &expr);
    }
}

#[test]
fn generated_queries_roundtrip() {
    let mut rng = Rng(0xD1B5_4A32_D192_ED03);
    for _ in 0..500 {
        let src = gen_query(&mut rng);
        let query = parse_query(&src)
            .unwrap_or_else(|e| panic!("generated query failed: {}\n{src}", e.render(&src)));
        check_query_roundtrip(&src, &query);
    }
}

const SOUP: &[&str] = &[
    "1", "2.5", "1e", ".", "\"s\"", "\"", "'T'", "'", "T", "[c]", "[", "]", "(", ")", "{", "}",
    ",", "+", "-", "*", "/", "^", "&", "&&", "|", "||", "=", "==", "<>", "<", ">", "<=", ">=",
    "IN", "NOT", "TRUE", "FALSE", "VAR", "x", "RETURN", "SUM", "DESC", "@p", "@", "dt\"d\"", "--",
    "//", "/*", "*/", "\n", "BLANK", "Year", "EVALUATE", "DEFINE", "MEASURE", "ORDER", "BY",
    "START", "AT", "TABLE", "COLUMN", "FUNCTION", "=>", ":", "é", "$", "\u{feff}",
];

#[test]
fn random_token_soup_never_panics() {
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    let mut parsed = 0;
    for _ in 0..20_000 {
        let len = rng.below(14);
        let mut src = String::new();
        for _ in 0..len {
            src.push_str(rng.pick(SOUP));
            src.push_str(rng.pick(&[" ", " ", ""]));
        }
        let tokens = tokenize(&src);
        let joined: String = tokens.iter().map(|t| t.text(&src)).collect();
        assert_eq!(joined, src, "lexer must be lossless");
        if let Ok(expr) = parse_expression(&src) {
            parsed += 1;
            check_expr_roundtrip(&src, &expr);
        }
        if let Ok(query) = parse_query(&src) {
            check_query_roundtrip(&src, &query);
        }
    }
    assert!(parsed > 100, "soup too rarely valid: {parsed}");
}

#[test]
fn truncated_inputs_never_panic() {
    let mut all = data_files("");
    all.extend(data_files("extra"));
    all.extend(data_files("queries"));
    for (_, src) in &all {
        for (i, _) in src.char_indices() {
            let prefix = &src[..i];
            if let Ok(expr) = parse_expression(prefix) {
                check_expr_roundtrip(prefix, &expr);
            }
            if let Ok(query) = parse_query(prefix) {
                check_query_roundtrip(prefix, &query);
            }
            if let Err(e) = parse_expression(prefix) {
                assert!(e.span.end <= prefix.len());
                let _ = e.render(prefix);
            }
        }
    }
}
