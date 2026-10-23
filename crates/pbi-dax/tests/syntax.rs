#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Parser, lexer, AST-shape, reference and error tests.

mod common;

use common::{data_files, sexp};
use pbi_dax::syntax::*;
use pretty_assertions::assert_eq;

fn parse(src: &str) -> Expr {
    parse_expression(src).unwrap_or_else(|e| panic!("{}\n{src}", e.render(src)))
}

fn s(src: &str) -> String {
    sexp(&parse(src))
}

fn err(src: &str) -> ParseError {
    match parse_expression(src) {
        Ok(e) => panic!("expected error for {src:?}, got {}", sexp(&e)),
        Err(e) => e,
    }
}

// ---- fixtures ---------------------------------------------------------------

#[test]
fn all_fixture_expressions_parse() {
    let files = data_files("");
    assert!(files.len() >= 11, "fixture copies missing");
    for (path, src) in files.iter().chain(data_files("extra").iter()) {
        let expr = parse_expression(src)
            .unwrap_or_else(|e| panic!("{}: {}", path.display(), e.render(src)));
        assert!(!expr.text(src).is_empty());
    }
}

/// Re-extracts single-line `measure`/`column` expressions straight from the
/// example's TMDL model, checks that they parse and that every single-line
/// copy under tests/data is one of them, so the copies cannot drift silently.
#[test]
fn tmdl_fixture_expressions_parse() {
    let dir = common::data::reference_root()
        .join("examples/acme-outfitters/AcmeOutfitters.SemanticModel/definition/tables");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        println!("SKIP: {} missing", dir.display());
        return;
    };
    let mut expressions = Vec::new();
    for entry in entries {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "tmdl") {
            continue;
        }
        for line in std::fs::read_to_string(&path).unwrap().lines() {
            if let Some(expr) = tmdl_decl_expr(line.trim()) {
                parse_expression(expr)
                    .unwrap_or_else(|e| panic!("{}: {}", path.display(), e.render(expr)));
                expressions.push(expr.to_owned());
            }
        }
    }
    assert_eq!(
        expressions.len(),
        40,
        "31 single-line measures + 9 single-line calculated columns"
    );
    let mut copies = 0;
    for (path, src) in data_files("") {
        let src = src.trim();
        if src.lines().count() == 1 {
            assert!(
                expressions.iter().any(|e| e == src),
                "{} is not in the model",
                path.display()
            );
            copies += 1;
        }
    }
    assert_eq!(copies, 14);
}

fn tmdl_decl_expr(line: &str) -> Option<&str> {
    let rest = line
        .strip_prefix("measure ")
        .or_else(|| line.strip_prefix("column "))?;
    let after_name = if let Some(quoted) = rest.strip_prefix('\'') {
        let mut i = 0;
        loop {
            let j = quoted[i..].find('\'')? + i;
            if quoted[j + 1..].starts_with('\'') {
                i = j + 2;
            } else {
                break &quoted[j + 1..];
            }
        }
    } else {
        &rest[rest.find(' ')?..]
    };
    after_name
        .trim_start()
        .strip_prefix('=')
        .map(str::trim)
        .filter(|e| !e.is_empty())
}

#[test]
fn fixture_ast_shapes() {
    let read = |name: &str| {
        data_files("")
            .into_iter()
            .find(|(p, _)| p.file_stem().unwrap() == name)
            .unwrap()
            .1
    };
    assert_eq!(
        s(&read("orders_completed_sales")),
        r#"(CALCULATE col:[Total Sales] (KEEPFILTERS (= col:Orders[Status] "Completed")))"#
    );
    assert_eq!(
        s(&read("orders_unknown_product_orders")),
        "(+ (COUNTROWS (FILTER table:Orders (ISBLANK (RELATED col:Products[Product Name])))) 0)"
    );
    assert_eq!(
        s(&read("orders_margin_pct")),
        "(DIVIDE col:[Gross Margin] col:[Total Sales])"
    );
    assert_eq!(
        s(&read("orders_last_order_date")),
        "(MAX col:Orders[Order Date])"
    );
    assert_eq!(
        s(&read("orders_col_is_returned")),
        r#"(IF (ISEMPTY (RELATEDTABLE table:Returns)) "No" "Yes")"#
    );
    assert_eq!(
        s(&read("table_products_never_sold")),
        "(var [SoldProducts (SELECTCOLUMNS (SUMMARIZE table:Orders col:Orders[Product ID]) \
         \"Product ID\" col:Orders[Product ID])] [Catalog (SELECTCOLUMNS (ALL table:Products) \
         \"Product ID\" col:Products[Product ID])] [NeverSold (EXCEPT $Catalog $SoldProducts)] \
         (SELECTCOLUMNS (FILTER (ALL table:Products) (&& (IN col:Products[Product ID] $NeverSold) \
         (ISEMPTY (RELATEDTABLE table:Orders)))) \"Product ID\" col:Products[Product ID] \
         \"Product\" col:Products[Product Name] \"Category\" col:Products[Category] \
         \"List Price\" col:Products[List Price]))"
    );
}

#[test]
fn spans_recover_source_text() {
    let src = r#"CALCULATE(COUNTROWS(Orders), Orders[Status] = "Completed") + 0"#;
    let expr = parse(src);
    assert_eq!(expr.text(src), src);
    let ExprKind::Binary { lhs, rhs, .. } = &expr.kind else {
        panic!()
    };
    assert_eq!(rhs.text(src), "0");
    let (name, args) = lhs.as_call().unwrap();
    assert_eq!(name, "CALCULATE");
    assert_eq!(args[0].text(src), "COUNTROWS(Orders)");
    assert_eq!(args[1].text(src), r#"Orders[Status] = "Completed""#);
}

// ---- references -------------------------------------------------------------

#[test]
fn references_of_calculated_table() {
    let (_, src) = data_files("")
        .into_iter()
        .find(|(p, _)| p.ends_with("table_products_never_sold.dax"))
        .unwrap();
    let expr = parse(&src);
    let col = |t: &str, c: &str| Reference::Column {
        table: t.into(),
        column: c.into(),
    };
    let f = |n: &str| Reference::Function(n.into());
    assert_eq!(
        references(&expr),
        vec![
            f("SELECTCOLUMNS"),
            f("SUMMARIZE"),
            Reference::Table("Orders".into()),
            col("Orders", "Product ID"),
            f("ALL"),
            Reference::Table("Products".into()),
            col("Products", "Product ID"),
            f("EXCEPT"),
            f("FILTER"),
            f("ISEMPTY"),
            f("RELATEDTABLE"),
            col("Products", "Product Name"),
            col("Products", "Category"),
            col("Products", "List Price"),
        ]
    );
    // Variables (`NeverSold`, `Catalog`, …) are not model references.
    assert_eq!(referenced_tables(&expr), vec!["Orders", "Products"]);

    let sites = reference_sites(&expr);
    let (_, span) = sites
        .iter()
        .find(|(r, _)| *r == col("Products", "Product Name"))
        .unwrap();
    assert_eq!(span.text(&src), "Products[Product Name]");
}

#[test]
fn references_measures_vars_and_case() {
    let expr = parse("DIVIDE([Gross Margin], [Total Sales], 0)");
    assert_eq!(
        references(&expr),
        vec![
            Reference::Function("DIVIDE".into()),
            Reference::Measure("Gross Margin".into()),
            Reference::Measure("Total Sales".into()),
        ]
    );
    // Variables, enum keywords and parameters are not model references; names
    // are de-duplicated case-insensitively.
    let expr = parse(
        "VAR t = 1 RETURN t + COUNTROWS(Sales) + COUNTROWS(SALES) + TOPN(1, x, [m], DESC) + @p",
    );
    assert_eq!(
        references(&expr),
        vec![
            Reference::Function("COUNTROWS".into()),
            Reference::Table("Sales".into()),
            Reference::Function("TOPN".into()),
            Reference::Table("x".into()),
            Reference::Measure("m".into()),
        ]
    );
}

// ---- precedence ---------------------------------------------------------------

#[test]
fn operator_precedence() {
    let cases = [
        ("1 + 2 * 3 ^ 2", "(+ 1 (* 2 (^ 3 2)))"),
        // Microsoft docs: `=-2^2` is -4, i.e. `^` binds tighter than sign.
        ("-2 ^ 2", "(neg (^ 2 2))"),
        ("(-2) ^ 2", "(^ (paren (neg 2)) 2)"),
        ("2 ^ 3 ^ 2", "(^ (^ 2 3) 2)"),
        ("2 ^ -1", "(^ 2 (neg 1))"),
        ("-2 * 3", "(* (neg 2) 3)"),
        ("- - 1", "(neg (neg 1))"),
        ("1 - -1", "(- 1 (neg 1))"),
        ("+[x]", "(pos col:[x])"),
        ("1 - 2 - 3", "(- (- 1 2) 3)"),
        ("8 / 4 / 2", "(/ (/ 8 4) 2)"),
        ("1 + 2 / 3 - 4", "(- (+ 1 (/ 2 3)) 4)"),
        (r#""a" & 1 + 2"#, r#"(& "a" (+ 1 2))"#),
        ("1 + 2 & 3 * 4", "(& (+ 1 2) (* 3 4))"),
        (r#"[A] & [B] = "xy""#, r#"(= (& col:[A] col:[B]) "xy")"#),
        (r#"[C] IN { "a", "b" }"#, r#"(IN col:[C] {("a") ("b")})"#),
        ("[C] in {1} && [D]", "(&& (IN col:[C] {(1)}) col:[D])"),
        ("1 + 1 IN {2}", "(IN (+ 1 1) {(2)})"),
        ("NOT [x] && [y]", "(&& (not col:[x]) col:[y])"),
        ("NOT [a] = 1", "(not (= col:[a] 1))"),
        ("NOT [a] IN {1}", "(not (IN col:[a] {(1)}))"),
        ("NOT NOT TRUE", "(not (not TRUE))"),
        // `NOT(...)` is the function form.
        ("NOT(TRUE) = FALSE", "(= (NOT TRUE) FALSE)"),
        ("[a] || [b] && [c]", "(|| col:[a] (&& col:[b] col:[c]))"),
        ("[a] && [b] || [c]", "(|| (&& col:[a] col:[b]) col:[c])"),
        ("1 < 2 = TRUE", "(= (< 1 2) TRUE)"),
        ("[a] == BLANK()", "(== col:[a] (BLANK))"),
        (
            "[a] <> 1 && [b] >= 2 || NOT [c] <= 3",
            "(|| (&& (<> col:[a] 1) (>= col:[b] 2)) (not (<= col:[c] 3)))",
        ),
        ("[a] > 1 && [b] < 2", "(&& (> col:[a] 1) (< col:[b] 2))"),
    ];
    for (src, expected) in cases {
        assert_eq!(s(src), expected, "{src}");
    }
}

// ---- VAR / RETURN ----------------------------------------------------------

#[test]
fn var_return_nesting_and_scoping() {
    assert_eq!(
        s("VAR a = 1 VAR b = VAR c = a RETURN c + 1 RETURN a + b"),
        "(var [a 1] [b (var [c $a] (+ $c 1))] (+ $a $b))"
    );
    // A variable shadows a table of the same name; quoting forces the table.
    assert_eq!(
        s("VAR Orders = 1 RETURN Orders + COUNTROWS('Orders')"),
        "(var [Orders 1] (+ $Orders (COUNTROWS table:Orders)))"
    );
    // Scope ends with the block.
    assert_eq!(
        s("(VAR t = 1 RETURN t) + COUNTROWS(t)"),
        "(+ (paren (var [t 1] $t)) (COUNTROWS table:t))"
    );
    // Names are case-insensitive; a variable is not in scope in its own body.
    assert_eq!(s("VAR _X = 1 RETURN _x"), "(var [_X 1] $_x)");
    assert_eq!(s("VAR t = t RETURN t"), "(var [t table:t] $t)");
    // A VAR block as an operand swallows the rest of the expression.
    assert_eq!(
        s("1 + VAR x = 2 RETURN x * 3"),
        "(+ 1 (var [x 2] (* $x 3)))"
    );
    let src = "VAR x = 1\nVAR y = 2\nRETURN x + y";
    let expr = parse(src);
    let ExprKind::Var { bindings, .. } = &expr.kind else {
        panic!()
    };
    assert_eq!(bindings[1].name_span.text(src), "y");
    assert_eq!(bindings[1].expr.text(src), "2");

    let e = err("VAR x = 1");
    assert_eq!(e.expected, vec!["VAR", "RETURN"]);
    assert_eq!(e.span, Span::empty(9));
    let e = err("VAR RETURN = 1 RETURN 1");
    assert_eq!(e.expected, vec!["variable name"]);
}

// ---- table constructors -------------------------------------------------------

#[test]
fn table_constructors() {
    assert_eq!(s("{1, 2, 3}"), "{(1) (2) (3)}");
    assert_eq!(s(r#"{(1, "a"), (2, "b")}"#), r#"{(1 "a") (2 "b")}"#);
    assert_eq!(s("{}"), "{}");
    assert_eq!(s("{(1)}"), "{((paren 1))}");
    // `{"a", 1}` is a one-column, two-row table; DATATABLE's evaluator
    // re-interprets each nested constructor as a row.
    assert_eq!(
        s(r#"DATATABLE("Name", STRING, {{"a", 1}, {"b", 2}})"#),
        r#"(DATATABLE "Name" kw:STRING {({("a") (1)}) ({("b") (2)})})"#
    );
    assert_eq!(
        s("('T'[A], 'T'[B]) IN {(1, 2), (3, 4)}"),
        "(IN (tuple col:T[A] col:T[B]) {(1 2) (3 4)})"
    );
    let ExprKind::TableConstructor { rows } = parse(r#"{(1, "a"), (2, "b")}"#).kind else {
        panic!()
    };
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.len() == 2));
    assert_eq!(err("{1, }").expected, vec!["expression"]);
    assert_eq!(err("{1 2}").expected, vec!["','", "'}'"]);
}

// ---- names, literals, calls ---------------------------------------------------

#[test]
fn quoted_names_and_escapes() {
    let expr = parse("'It''s'[A]]B]");
    assert_eq!(
        expr.kind,
        ExprKind::ColumnRef {
            table: Some("It's".into()),
            column: "A]B".into()
        }
    );
    assert_eq!(print_expr(&expr), "'It''s'[A]]B]");

    let src = r#""say ""hi""""#;
    assert_eq!(parse(src).kind, ExprKind::String(r#"say "hi""#.into()));
    assert_eq!(print_expr(&parse(src)), src);

    assert_eq!(s("'Web Sessions'"), "table:Web Sessions");
    assert_eq!(print_expr(&parse("'Orders'")), "Orders");
    assert_eq!(print_expr(&parse("'Table'")), "'Table'");
    assert_eq!(print_expr(&parse("'Year'[x]")), "'Year'[x]");
    assert_eq!(s("Données[Montant]"), "col:Données[Montant]");
    assert_eq!(s("[]"), "col:[]");
    assert_eq!(s(r#"dt"2024-01-31""#), r#"dt"2024-01-31""#);
    assert_eq!(s("@MinCount"), "@MinCount");
    // Whitespace between table and column is tolerated.
    assert_eq!(s("Sales [Amount]"), "col:Sales[Amount]");
    // Reserved words used as table names before a bracket.
    assert_eq!(s("Order[Id]"), "col:Order[Id]");
}

#[test]
fn numbers() {
    let n = |src: &str| match parse(src).kind {
        ExprKind::Number(n) => n,
        other => panic!("{other:?}"),
    };
    assert_eq!(n("42"), Number::Integer(42));
    assert_eq!(n("1.5"), Number::Decimal(1.5));
    assert_eq!(n(".5"), Number::Decimal(0.5));
    assert_eq!(n("1."), Number::Decimal(1.0));
    assert_eq!(n("1e3"), Number::Decimal(1000.0));
    assert_eq!(n("2.5E-3"), Number::Decimal(0.0025));
    assert_eq!(n("1.e2"), Number::Decimal(100.0));
    assert_eq!(n("99999999999999999999"), Number::Decimal(1e20));
}

#[test]
fn calls_keywords_and_omitted_args() {
    let expr = parse("Calculate ( [m] )");
    let ExprKind::Call { name, raw_name, .. } = &expr.kind else {
        panic!()
    };
    assert_eq!(
        (name.as_str(), raw_name.as_str()),
        ("CALCULATE", "Calculate")
    );
    assert_eq!(
        s("PERCENTILE.INC(T[x], 0.5)"),
        "(PERCENTILE.INC col:T[x] 0.5)"
    );
    assert_eq!(s("T.DIST(0.5, 2, TRUE)"), "(T.DIST 0.5 2 TRUE)");
    assert_eq!(s("TRUE() && FALSE ()"), "(&& TRUE FALSE)");
    assert_eq!(s("F()"), "(F)");
    assert_eq!(s("F(a, )"), "(F table:a _)");
    assert_eq!(s("F(,)"), "(F _ _)");

    let src = "RANKX(ALL(T), [m], , desc, Dense)";
    let expr = parse(src);
    assert_eq!(s(src), "(RANKX (ALL table:T) col:[m] _ kw:DESC kw:DENSE)");
    let (_, args) = expr.as_call().unwrap();
    assert_eq!(args[2].span, Span::empty(19));
    assert_eq!(args[2].kind, ExprKind::Omitted);

    assert_eq!(
        s("DATEADD('Date'[Date], -1, Year)"),
        "(DATEADD col:Date[Date] (neg 1) kw:YEAR)"
    );
    // Enum words are still functions, columns or variables in those positions.
    assert_eq!(s("YEAR('Date'[Date])"), "(YEAR col:Date[Date])");
    assert_eq!(s("Year[x]"), "col:Year[x]");
    assert_eq!(s("VAR Year = 1 RETURN Year"), "(var [Year 1] $Year)");
    assert_eq!(s("'Year'"), "table:Year");
    // Contextual query keywords are ordinary identifiers in expressions.
    assert_eq!(s("COUNTROWS(Table)"), "(COUNTROWS table:Table)");
}

#[test]
fn comments_are_trivia() {
    let src = "// leading\n1 + /* inline\n comment */ 2 -- trailing";
    assert_eq!(s(src), "(+ 1 2)");
    assert_eq!(parse(src).text(src), "1 + /* inline\n comment */ 2");
    assert_eq!(s("5 --3"), "5");
    assert_eq!(s("5 - -3"), "(- 5 (neg 3))");

    let kinds: Vec<TokenKind> = tokenize(src).iter().map(|t| t.kind).collect();
    assert!(kinds.contains(&TokenKind::LineComment));
    assert!(kinds.contains(&TokenKind::BlockComment));
    assert_eq!(err("1 /* open").message, "unterminated block comment");
}

// ---- lexer --------------------------------------------------------------------

#[test]
fn lexer_tokens() {
    let src =
        "x 1 1.5 .5 1e3 2.5E-3 1e PERCENTILE.INC @p dt\"2020-01-01\" 'a''b' [c]]d] \"e\"\"f\"";
    let toks: Vec<(TokenKind, String)> = tokenize(src)
        .into_iter()
        .filter(|t| !t.is_trivia())
        .map(|t| (t.kind, t.value(src)))
        .collect();
    use TokenKind as K;
    let expected = [
        (K::Ident, "x"),
        (K::Number, "1"),
        (K::Number, "1.5"),
        (K::Number, ".5"),
        (K::Number, "1e3"),
        (K::Number, "2.5E-3"),
        (K::Number, "1"),
        (K::Ident, "e"),
        (K::Ident, "PERCENTILE.INC"),
        (K::Parameter, "p"),
        (K::DateTime, "2020-01-01"),
        (K::QuotedIdent, "a'b"),
        (K::BracketIdent, "c]d"),
        (K::String, "e\"f"),
    ];
    let expected: Vec<(TokenKind, String)> =
        expected.iter().map(|(k, v)| (*k, v.to_string())).collect();
    assert_eq!(toks, expected);

    let ops = "+ - * / ^ & && || = == <> < > <= >= ( ) { } , . : =>";
    let kinds: Vec<TokenKind> = tokenize(ops)
        .into_iter()
        .filter(|t| !t.is_trivia())
        .map(|t| t.kind)
        .collect();
    assert_eq!(
        kinds,
        vec![
            K::Plus,
            K::Minus,
            K::Star,
            K::Slash,
            K::Caret,
            K::Amp,
            K::AmpAmp,
            K::PipePipe,
            K::Eq,
            K::EqEq,
            K::Neq,
            K::Lt,
            K::Gt,
            K::Le,
            K::Ge,
            K::LParen,
            K::RParen,
            K::LBrace,
            K::RBrace,
            K::Comma,
            K::Dot,
            K::Colon,
            K::Arrow,
        ]
    );

    let src = "return Evaluate order";
    let kws: Vec<Option<Keyword>> = tokenize(src)
        .iter()
        .filter(|t| !t.is_trivia())
        .map(|t| t.keyword(src))
        .collect();
    assert_eq!(
        kws,
        vec![
            Some(Keyword::Return),
            Some(Keyword::Evaluate),
            Some(Keyword::Order)
        ]
    );
}

// ---- errors -------------------------------------------------------------------

#[test]
fn error_spans_and_messages() {
    let e = err("SUM(");
    assert_eq!(e.message, "unexpected end of input");
    assert_eq!(e.span, Span::empty(4));
    assert_eq!(e.expected, vec!["expression"]);

    let e = err("1 + * 2");
    assert_eq!(e.message, "unexpected '*'");
    assert_eq!(e.span, Span::new(4, 5));
    assert_eq!(e.to_string(), "unexpected '*', expected expression");

    let e = err("CALCULATE([x] [y])");
    assert_eq!(e.span, Span::new(14, 17));
    assert_eq!(e.expected, vec!["','", "')'"]);
    assert_eq!(
        e.to_string(),
        "unexpected column/measure reference [y], expected ',' or ')'"
    );

    let e = err("1 2");
    assert_eq!(e.span, Span::new(2, 3));
    assert_eq!(e.expected, vec!["end of input"]);

    let e = err("\"abc");
    assert_eq!(
        (e.message.as_str(), e.span),
        ("unterminated string literal", Span::new(0, 4))
    );
    let e = err("COUNTROWS('Web Sessions)");
    assert_eq!(
        (e.message.as_str(), e.span),
        ("unterminated quoted name", Span::new(10, 24))
    );
    assert_eq!(err("[x").message, "unterminated bracketed name");
    assert_eq!(err("1 | 2").span, Span::new(2, 3));
    let e = err("1 $ 2");
    assert_eq!(
        (e.message.as_str(), e.span),
        ("unexpected character", Span::new(2, 3))
    );
    assert_eq!(err("").message, "unexpected end of input");
    assert_eq!(err("RETURN 1").message, "unexpected keyword RETURN");
    assert_eq!(err("TRUE(1)").expected, vec!["')'"]);

    let src = "IF(\n  [a] = 1,\n  2 3\n)";
    let e = err(src);
    assert_eq!(e.line_col(src), (3, 5));
    assert_eq!(
        e.render(src),
        "line 3, column 5: unexpected number 3, expected ',' or ')'"
    );
}

#[test]
fn line_col_helper() {
    let src = "é\nab";
    assert_eq!(line_col(src, 0), (1, 1));
    assert_eq!(line_col(src, 1), (1, 1)); // inside a multi-byte char
    assert_eq!(line_col(src, 2), (1, 2));
    assert_eq!(line_col(src, 4), (2, 2));
    assert_eq!(line_col(src, 100), (2, 3));
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    // At the depth limit everything fits in ~0.5 MiB of stack even in debug
    // builds; run on a 1 MiB thread (half the default) to keep headroom honest.
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(|| {
            let n = 100_000;
            let too_deep = [
                format!("{}1{}", "(".repeat(n), ")".repeat(n)),
                format!("{}1", "- ".repeat(n)),
                format!("{}TRUE", "NOT ".repeat(n)),
                "F(".repeat(n),
                "{".repeat(n),
                format!("{}1{}", "VAR a = ".repeat(n), " RETURN a".repeat(n)),
                format!("1{}", " + 1".repeat(300)),
                format!("1{}", " ^ 1".repeat(5_000)),
            ];
            for src in &too_deep {
                let e = parse_expression(src).unwrap_err();
                assert!(e.message.contains("nested too deeply"), "{}", e.message);
            }
            let q = format!("EVALUATE {}1{}", "(".repeat(n), ")".repeat(n));
            assert!(parse_query(&q).is_err());

            // Within the limits everything parses, prints and re-parses.
            let ok = [
                format!("{}1{}", "(".repeat(200), ")".repeat(200)),
                format!("1{}", " + 1".repeat(250)),
                format!("{}1{}", "{(".repeat(120), ")}".repeat(120)),
                format!("{}1{}", "(".repeat(254), ")".repeat(254)),
                format!("{}1{}", "IF(TRUE, ".repeat(100), ")".repeat(100)),
            ];
            for src in &ok {
                let expr = parse_expression(src).unwrap();
                let printed = print_expr(&expr);
                assert!(parse_expression(&printed).unwrap().eq_ignoring_spans(&expr));
                let pretty = pretty_expr(&expr, &PrintOptions::default());
                assert!(parse_expression(&pretty).unwrap().eq_ignoring_spans(&expr));
                let _ = references(&expr);
                assert_eq!(expr.clone(), expr);
                assert!(!format!("{expr:?}").is_empty());
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

// ---- queries ------------------------------------------------------------------

#[test]
fn query_with_all_define_kinds() {
    let (_, src) = data_files("queries")
        .into_iter()
        .find(|(p, _)| p.ends_with("kpis.dax"))
        .unwrap();
    let q = parse_query(&src).unwrap_or_else(|e| panic!("{}", e.render(&src)));
    assert_eq!(q.defines.len(), 5);
    assert!(matches!(&q.defines[0], Define::Var { name, .. } if name == "_threshold"));
    match &q.defines[1] {
        Define::Measure {
            table, name, expr, ..
        } => {
            assert_eq!(
                (table.as_str(), name.as_str()),
                ("Orders", "Completed Share")
            );
            assert_eq!(
                expr.text(&src),
                "DIVIDE ( [Completed Sales], [Total Sales], 0 )"
            );
        }
        other => panic!("{other:?}"),
    }
    match &q.defines[2] {
        Define::Table { name, expr, .. } => {
            assert_eq!(name, "Small Orders");
            // The DEFINE VAR is in scope for later definitions.
            assert_eq!(
                sexp(expr),
                "(FILTER table:Orders (< col:Orders[Quantity] $_threshold))"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        matches!(&q.defines[3], Define::Column { table, name, .. } if table == "Products" && name == "Upper Name")
    );
    match &q.defines[4] {
        Define::Function {
            name, params, body, ..
        } => {
            assert_eq!(name, "Local.AddTax");
            assert_eq!(params.len(), 2);
            assert_eq!(params[0].name, "amount");
            assert_eq!(params[0].type_hint.as_deref(), Some("SCALAR NUMERIC VAL"));
            assert_eq!(params[1].type_hint, None);
            assert_eq!(sexp(body), "(* $amount (paren (+ 1 $rate)))");
        }
        other => panic!("{other:?}"),
    }

    assert_eq!(q.evaluates.len(), 3);
    let ev = &q.evaluates[0];
    assert!(
        ev.expr
            .as_call()
            .is_some_and(|(n, _)| n == "SUMMARIZECOLUMNS")
    );
    let order: Vec<(String, SortDir)> = ev
        .order_by
        .iter()
        .map(|(e, d)| (e.text(&src).to_string(), *d))
        .collect();
    assert_eq!(
        order,
        vec![
            ("Orders[Status]".to_string(), SortDir::Desc),
            ("[Orders]".to_string(), SortDir::Asc),
        ]
    );
    assert_eq!(ev.start_at.len(), 1);
    assert_eq!(ev.start_at[0].kind, ExprKind::String("Pending".into()));
    assert!(ev.span.text(&src).starts_with("EVALUATE"));
    assert!(ev.span.text(&src).ends_with("START AT \"Pending\""));
    assert_eq!(sexp(&q.evaluates[1].expr), "table:Small Orders");
    assert_eq!(sexp(&q.evaluates[2].expr), "{($_threshold)}");
}

#[test]
fn query_clauses_and_errors() {
    let q = parse_query("evaluate T order by T[a] asc, T[b] desc start at 1, @p").unwrap();
    assert!(q.defines.is_empty());
    let ev = &q.evaluates[0];
    assert_eq!(
        ev.order_by.iter().map(|(_, d)| *d).collect::<Vec<_>>(),
        vec![SortDir::Asc, SortDir::Desc]
    );
    assert_eq!(ev.start_at.len(), 2);

    let err = |src: &str| parse_query(src).unwrap_err();
    assert_eq!(
        err("DEFINE EVALUATE 1").expected,
        vec!["MEASURE", "VAR", "TABLE", "COLUMN", "FUNCTION"]
    );
    assert_eq!(
        err("EVALUATE T START AT 1").message,
        "START AT requires a preceding ORDER BY"
    );
    assert_eq!(err("MEASURE T[m] = 1").expected, vec!["DEFINE", "EVALUATE"]);
    assert_eq!(
        err("DEFINE MEASURE T[m] = 1").expected,
        vec!["MEASURE", "VAR", "TABLE", "COLUMN", "FUNCTION", "EVALUATE"]
    );
    assert_eq!(
        err("DEFINE MEASURE [m] = 1 EVALUATE {1}").expected,
        vec!["table name"]
    );
    assert_eq!(err("EVALUATE T ORDER T[a]").expected, vec!["BY"]);
    assert_eq!(err("EVALUATE").expected, vec!["expression"]);
    assert_eq!(err("EVALUATE {1} 2").expected, vec!["end of input"]);
}

#[test]
fn function_definitions() {
    let def =
        parse_function_definition("(amount : NUMERIC, t : TABLE EXPR) => amount * COUNTROWS(t)")
            .unwrap();
    assert_eq!(def.params.len(), 2);
    assert_eq!(def.params[1].type_hint.as_deref(), Some("TABLE EXPR"));
    assert_eq!(sexp(&def.body), "(* $amount (COUNTROWS $t))");
    assert_eq!(
        parse_function_definition("() => 1").unwrap().params.len(),
        0
    );
    assert_eq!(
        parse_function_definition("(a :) => 1")
            .unwrap_err()
            .expected,
        vec!["parameter type"]
    );
    assert_eq!(
        parse_function_definition("(a) 1").unwrap_err().expected,
        vec!["'=>'"]
    );
}
