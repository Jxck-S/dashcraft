#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Formatter tests (`service::format`): layouts, comments, casing, errors,
//! and idempotency and parse preservation over the test data, generated
//! expressions and queries, injected comments and random token soup.

mod common;

use common::data_files;
use common::generate::{Rng, SOUP, gen_expr, gen_query};
use pbi_dax::service::{
    Casing, DiagnosticCode, FormatError, FormatOptions, MAX_INDENT_WIDTH, SourceKind, format,
};
use pbi_dax::syntax::{
    MAX_DEPTH, TokenKind, parse_expression, parse_query, print_expr, print_query, tokenize,
};
use pretty_assertions::assert_eq;

/// Formats `src` (expression or query, detected), checks that formatting the
/// result again changes nothing, and returns it.
fn fmt_with(src: &str, opts: &FormatOptions) -> String {
    let kind = SourceKind::detect(src);
    let out = format(src, kind, opts).unwrap_or_else(|e| panic!("{e}\nsource:\n{src}"));
    let again = format(&out, kind, opts).unwrap_or_else(|e| panic!("{e}\nformatted:\n{out}"));
    assert_eq!(again, out, "not idempotent; source:\n{src}");
    out
}

fn fmt(src: &str) -> String {
    fmt_with(src, &FormatOptions::default())
}

fn fmt_width(src: &str, max_width: usize) -> String {
    fmt_with(
        src,
        &FormatOptions {
            max_width,
            ..FormatOptions::default()
        },
    )
}

// ---- layout ----------------------------------------------------------------

#[test]
fn short_calls_stay_on_one_line() {
    assert_eq!(
        fmt("calculate( sum(Sales[Amount]) , Stores[Region]=\"West\" )"),
        "CALCULATE(SUM(Sales[Amount]), Stores[Region] = \"West\")"
    );
    assert_eq!(fmt("RANKX(ALL(T),[x],,DESC)"), "RANKX(ALL(T), [x], , DESC)");
    assert_eq!(fmt("  1+2*3  "), "1 + 2 * 3");
    assert_eq!(fmt("TRUE()&&false"), "TRUE() && FALSE");
}

#[test]
fn long_calls_break_one_argument_per_line() {
    let src = "CALCULATE(SUMX(FILTER(Sales, Sales[Quantity] > 10 && Sales[Price] * \
               Sales[Quantity] > 1000), Sales[Quantity] * Sales[Price]), ALL(Dates), \
               Dates[Year] = 2024)";
    assert_eq!(
        fmt(src),
        "CALCULATE(
    SUMX(
        FILTER(
            Sales,
            Sales[Quantity] > 10 && Sales[Price] * Sales[Quantity] > 1000
        ),
        Sales[Quantity] * Sales[Price]
    ),
    ALL(Dates),
    Dates[Year] = 2024
)"
    );
    assert_eq!(
        fmt_width(
            "CALCULATE(SUM(Sales[Amount]), Stores[Region] = \"West\")",
            30
        ),
        "CALCULATE(
    SUM(Sales[Amount]),
    Stores[Region] = \"West\"
)"
    );
}

#[test]
fn operator_chains_break_before_operators_and_nest_by_precedence() {
    assert_eq!(
        fmt_width(
            "Sales[Quantity] > 10 && Sales[Price] * Sales[Quantity] > 1000",
            30
        ),
        "Sales[Quantity] > 10
    && Sales[Price]
            * Sales[Quantity]
        > 1000"
    );
    assert_eq!(
        fmt_width("[Alpha] + [Beta] - [Gamma] + [Delta]", 20),
        "[Alpha]
    + [Beta]
    - [Gamma]
    + [Delta]"
    );
}

#[test]
fn var_blocks_put_return_on_its_own_line() {
    assert_eq!(
        fmt("var a=1 var b=a+1 return a+b"),
        "VAR a = 1
VAR b = a + 1
RETURN
    a + b"
    );
    assert_eq!(
        fmt(
            "VAR LongName = CALCULATE(SUM(Sales[Amount]), Stores[Region] = \"West\", \
             Dates[Year] = 2024) RETURN LongName"
        ),
        "VAR LongName =
    CALCULATE(SUM(Sales[Amount]), Stores[Region] = \"West\", Dates[Year] = 2024)
RETURN
    LongName"
    );
    assert_eq!(
        fmt("SUMX(T, VAR x = T[a] RETURN x * 2)"),
        "SUMX(
    T,
    VAR x = T[a]
    RETURN
        x * 2
)"
    );
}

#[test]
fn table_constructors_and_tuples() {
    assert_eq!(fmt("{(1,\"a\"),(2,\"b\")}"), "{(1, \"a\"), (2, \"b\")}");
    assert_eq!(fmt("{ 1 , 2 , 3 }"), "{1, 2, 3}");
    assert_eq!(fmt("{}"), "{}");
    assert_eq!(
        fmt_width("{(1, \"alpha\"), (2, \"beta\")}", 20),
        "{
    (1, \"alpha\"),
    (2, \"beta\")
}"
    );
    assert_eq!(fmt("(T[a], T[b]) IN {(1, 2)}"), "(T[a], T[b]) IN {(1, 2)}");
}

#[test]
fn queries_lay_out_define_evaluate_order_by_and_start_at() {
    assert_eq!(
        fmt(
            "define measure Sales[Total]=sum(Sales[Amount]) var _year=2024 \
             evaluate summarizecolumns(Stores[Region],\"Total\",[Total]) \
             order by Stores[Region] desc, [Total] start at \"East\" evaluate {1,2}"
        ),
        "DEFINE
    MEASURE Sales[Total] = SUM(Sales[Amount])
    VAR _year = 2024

EVALUATE
    SUMMARIZECOLUMNS(Stores[Region], \"Total\", [Total])
ORDER BY Stores[Region] DESC, [Total]
START AT \"East\"

EVALUATE
    {1, 2}"
    );
    assert_eq!(fmt("EVALUATE Sales"), "EVALUATE\n    Sales");
    assert_eq!(
        fmt("DEFINE FUNCTION Twice = (x : numeric val, y) => x * 2 EVALUATE {Twice(2)}"),
        "DEFINE
    FUNCTION Twice = (x : NUMERIC VAL, y) => x * 2

EVALUATE
    {Twice(2)}"
    );
    assert_eq!(
        fmt_width(
            "DEFINE FUNCTION Twice = (x : NUMERIC VAL, y) => x * 2 EVALUATE {1}",
            30
        ),
        "DEFINE
    FUNCTION Twice = (
        x : NUMERIC VAL,
        y
    ) => x * 2

EVALUATE
    {1}"
    );
}

#[test]
fn unary_operators_never_merge_into_comments() {
    assert_eq!(fmt("-  -1 + 1 - -1"), "- -1 + 1 - -1");
    assert_eq!(fmt("- + 1"), "- +1");
    assert_eq!(fmt("-(-1)"), "-(-1)");
    assert_eq!(fmt("not ( 1 = 2 )"), "NOT(1 = 2)");
    assert_eq!(fmt("NOT [Flag]"), "NOT [Flag]");
    // `NOT (x)` parses as an operator and `NOT(x)` as a call: both survive.
    let unary = fmt("NOT (1) && x");
    assert!(parse_expression(&unary).is_ok(), "{unary}");
}

// ---- comments --------------------------------------------------------------

#[test]
fn comments_stay_where_they_were() {
    assert_eq!(
        fmt(
            "-- total sales\nvar a=SUM(Sales[Amount])   -- all rows\n\n/* blank line kept */\n\
             var b=2\nreturn a+b // sum"
        ),
        "-- total sales
VAR a = SUM(Sales[Amount]) -- all rows

/* blank line kept */
VAR b = 2
RETURN
    a + b // sum"
    );
    assert_eq!(
        fmt("CALCULATE(\n  [Sales], -- current\n  // all years\n  ALL(Dates)\n)"),
        "CALCULATE(
    [Sales], -- current
    // all years
    ALL(Dates)
)"
    );
    assert_eq!(fmt("1 + /* two */ 2"), "1 + /* two */ 2");
    assert_eq!(fmt("SUM(T[c]) -- total   "), "SUM(T[c]) -- total");
    assert_eq!(
        fmt("SUM(T[c])\n-- end\n/* really */"),
        "SUM(T[c])\n-- end\n/* really */"
    );
    assert_eq!(
        fmt("VAR a = -- note\n 1 RETURN a"),
        "VAR a = -- note
    1
RETURN
    a"
    );
    assert_eq!(
        fmt("EVALUATE -- rows\n{1}\n-- done"),
        "EVALUATE -- rows\n    {1}\n-- done"
    );
}

#[test]
fn blank_lines_between_vars_and_defines_are_kept_once() {
    assert_eq!(
        fmt("VAR a = 1\n\n\n\nVAR b = 2\nRETURN a + b"),
        "VAR a = 1\n\nVAR b = 2\nRETURN\n    a + b"
    );
    assert_eq!(
        fmt("DEFINE\nVAR a = 1\n\nVAR b = 2\nEVALUATE {a + b}"),
        "DEFINE\n    VAR a = 1\n\n    VAR b = 2\n\nEVALUATE\n    {a + b}"
    );
}

// ---- options ---------------------------------------------------------------

#[test]
fn casing_applies_to_keywords_and_built_in_functions_only() {
    let src = "Var x = sumx(sales, sales[amount]) Return if(x > 0 && TRUE, MyLib.Double(x), \
               blank())";
    assert_eq!(
        fmt(src),
        "VAR x = SUMX(sales, sales[amount])
RETURN
    IF(x > 0 && TRUE, MyLib.Double(x), BLANK())"
    );
    let lower = FormatOptions {
        keyword_case: Casing::Lower,
        function_case: Casing::Lower,
        ..FormatOptions::default()
    };
    assert_eq!(
        fmt_with(src, &lower),
        "var x = sumx(sales, sales[amount])
return
    if(x > 0 && true, MyLib.Double(x), blank())"
    );
    let keep = FormatOptions {
        keyword_case: Casing::Keep,
        function_case: Casing::Keep,
        ..FormatOptions::default()
    };
    assert_eq!(
        fmt_with(src, &keep),
        "Var x = sumx(sales, sales[amount])
Return
    if(x > 0 && TRUE, MyLib.Double(x), blank())"
    );
    let mixed = FormatOptions {
        keyword_case: Casing::Lower,
        function_case: Casing::Upper,
        ..FormatOptions::default()
    };
    assert_eq!(
        fmt_with("DATEADD(Dates[Date], -1, year) IN {1}", &mixed),
        "DATEADD(Dates[Date], -1, year) in {1}"
    );
    assert_eq!(
        fmt("info.tables()"),
        "INFO.TABLES()",
        "INFO functions are built-in"
    );
}

#[test]
fn indentation_uses_the_width_or_tabs() {
    let src = "VAR a = 1 RETURN a";
    let two = FormatOptions {
        indent_width: 2,
        ..FormatOptions::default()
    };
    assert_eq!(fmt_with(src, &two), "VAR a = 1\nRETURN\n  a");
    let tabs = FormatOptions {
        use_tabs: true,
        ..FormatOptions::default()
    };
    assert_eq!(fmt_with(src, &tabs), "VAR a = 1\nRETURN\n\ta");
    let huge = FormatOptions {
        indent_width: 1_000,
        ..FormatOptions::default()
    };
    assert_eq!(
        fmt_with(src, &huge),
        format!("VAR a = 1\nRETURN\n{}a", " ".repeat(MAX_INDENT_WIDTH))
    );
    let zero = FormatOptions {
        indent_width: 0,
        ..FormatOptions::default()
    };
    assert_eq!(fmt_with(src, &zero), "VAR a = 1\nRETURN\na");
}

#[test]
fn blank_and_crlf_input() {
    for kind in [SourceKind::Expression, SourceKind::Query] {
        assert_eq!(
            format("", kind, &FormatOptions::default()),
            Ok(String::new())
        );
        assert_eq!(
            format(" \r\n\t", kind, &FormatOptions::default()),
            Ok(String::new())
        );
    }
    assert_eq!(
        fmt("VAR a = 1 -- one\r\nRETURN\r\n    a\r\n"),
        "VAR a = 1 -- one\nRETURN\n    a"
    );
}

#[test]
fn syntax_errors_are_diagnostics() {
    for (src, kind) in [
        ("SUM(", SourceKind::Expression),
        ("1 +", SourceKind::Expression),
        ("EVALUATE", SourceKind::Query),
        ("DEFINE MEASURE T[m] = EVALUATE {1}", SourceKind::Query),
        ("{1} EVALUATE", SourceKind::Expression),
    ] {
        match format(src, kind, &FormatOptions::default()) {
            Err(FormatError::Syntax(d)) => {
                assert_eq!(d.code, DiagnosticCode::Syntax, "{src}");
                assert!(d.span.end <= src.len(), "{src}: {d:?}");
                assert!(!d.message.is_empty());
            }
            other => panic!("{src}: expected a syntax error, got {other:?}"),
        }
    }
}

// ---- properties ------------------------------------------------------------

const WIDTHS: [usize; 4] = [120, 80, 40, 1];

fn option_matrix() -> Vec<FormatOptions> {
    let mut all = Vec::new();
    for max_width in WIDTHS {
        all.push(FormatOptions {
            max_width,
            ..FormatOptions::default()
        });
    }
    all.push(FormatOptions {
        max_width: 60,
        indent_width: 2,
        use_tabs: true,
        keyword_case: Casing::Lower,
        function_case: Casing::Keep,
    });
    all.push(FormatOptions {
        max_width: 100,
        indent_width: 3,
        use_tabs: false,
        keyword_case: Casing::Keep,
        function_case: Casing::Lower,
    });
    all
}

/// Non-whitespace tokens, keyword and identifier text in upper case.
fn significant(src: &str) -> Vec<String> {
    tokenize(src)
        .iter()
        .filter(|t| t.kind != TokenKind::Whitespace)
        .map(|t| {
            let text = t.text(src);
            match t.kind {
                TokenKind::Ident => text.to_ascii_uppercase(),
                TokenKind::LineComment => text.trim_end().to_owned(),
                _ => text.to_owned(),
            }
        })
        .collect()
}

/// `src` parses as `kind`: the formatted text is idempotent, parses to the
/// same tree, keeps every token and comment, and fits the width wherever a
/// line could break.
fn check_formats(src: &str, kind: SourceKind) {
    if src.trim().is_empty() {
        assert_eq!(
            format(src, kind, &FormatOptions::default()),
            Ok(String::new())
        );
        return;
    }
    for opts in option_matrix() {
        let out = match format(src, kind, &opts) {
            Ok(out) => out,
            Err(e) => panic!("{e}\noptions: {opts:?}\nsource:\n{src}"),
        };
        let again = format(&out, kind, &opts)
            .unwrap_or_else(|e| panic!("{e}\noptions: {opts:?}\nformatted:\n{out}"));
        assert_eq!(again, out, "not idempotent ({opts:?}); source:\n{src}");
        assert_eq!(
            significant(&out),
            significant(src),
            "tokens changed ({opts:?})"
        );
        // Casing changes the spelling of built-in names in the tree: compare
        // the canonical printing, upper-cased.
        let (a, b) = match kind {
            SourceKind::Expression => (
                print_expr(&parse_expression(src).unwrap()),
                print_expr(&parse_expression(&out).unwrap()),
            ),
            SourceKind::Query => (
                print_query(&parse_query(src).unwrap()),
                print_query(&parse_query(&out).unwrap()),
            ),
        };
        assert_eq!(
            a.to_ascii_uppercase(),
            b.to_ascii_uppercase(),
            "tree changed:\n{src}\n---\n{out}"
        );
        assert!(!out.contains('\r') && !out.ends_with('\n'));
        assert!(
            out.lines().all(|l| l.trim_end() == l),
            "trailing blanks:\n{out}"
        );
        assert!(!out.contains("\n\n\n"), "double blank line:\n{out}");
    }
}

#[test]
fn every_test_file_formats_idempotently() {
    let mut n = 0;
    for (path, src) in data_files("").iter().chain(data_files("extra").iter()) {
        assert!(parse_expression(src).is_ok(), "{}", path.display());
        check_formats(src, SourceKind::Expression);
        n += 1;
    }
    for (path, src) in data_files("queries") {
        assert!(parse_query(&src).is_ok(), "{}", path.display());
        check_formats(&src, SourceKind::Query);
        n += 1;
    }
    assert!(n > 20, "too few test files: {n}");
}

#[test]
fn default_width_keeps_lines_short_where_it_can() {
    let opts = FormatOptions::default();
    for (path, src) in data_files("") {
        let out = format(&src, SourceKind::Expression, &opts).unwrap();
        for line in out.lines() {
            let long_token = tokenize(line)
                .iter()
                .any(|t| t.kind != TokenKind::Whitespace && t.text(line).len() > 40);
            assert!(
                line.chars().count() <= opts.max_width || long_token,
                "{}: line too long:\n{line}",
                path.display()
            );
        }
    }
}

/// Puts a random comment or line break after some tokens of `src`.
fn sprinkle_comments(rng: &mut Rng, src: &str) -> String {
    let mut out = String::new();
    for token in tokenize(src) {
        out.push_str(token.text(src));
        match rng.below(10) {
            0 => out.push_str(" -- note\n"),
            1 => out.push_str("/* c */"),
            2 => out.push_str("\n\n"),
            3 => out.push_str(" // x\n\n"),
            _ => {}
        }
    }
    out
}

#[test]
fn generated_expressions_format_idempotently() {
    let mut rng = Rng(0x5851_F42D_4C95_7F2D);
    let mut commented_ok = 0;
    for _ in 0..1_500 {
        let depth = rng.below(6);
        let src = gen_expr(&mut rng, depth);
        check_formats(&src, SourceKind::Expression);
        let commented = sprinkle_comments(&mut rng, &src);
        if parse_expression(&commented).is_ok() {
            commented_ok += 1;
            check_formats(&commented, SourceKind::Expression);
        }
    }
    assert!(commented_ok > 1_000, "{commented_ok}");
}

#[test]
fn generated_queries_format_idempotently() {
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let mut commented_ok = 0;
    for _ in 0..300 {
        let src = gen_query(&mut rng);
        check_formats(&src, SourceKind::Query);
        let commented = sprinkle_comments(&mut rng, &src);
        if parse_query(&commented).is_ok() {
            commented_ok += 1;
            check_formats(&commented, SourceKind::Query);
        }
    }
    assert!(commented_ok > 200, "{commented_ok}");
}

#[test]
fn random_token_soup_never_panics_or_destabilizes() {
    let mut rng = Rng(0x7F4A_7C15_9E37_79B9);
    let mut formatted = 0;
    for _ in 0..20_000 {
        let mut src = String::new();
        for _ in 0..rng.below(14) {
            src.push_str(rng.pick(SOUP));
            src.push_str(rng.pick(&[" ", " ", ""]));
        }
        for kind in [SourceKind::Expression, SourceKind::Query] {
            match format(&src, kind, &FormatOptions::default()) {
                Ok(_) => {
                    formatted += 1;
                    check_formats(&src, kind);
                }
                Err(FormatError::Syntax(d)) => assert!(d.span.end <= src.len()),
                Err(FormatError::Unstable) => panic!("unstable on {src:?} ({kind:?})"),
            }
        }
    }
    assert!(formatted > 100, "soup too rarely valid: {formatted}");
}

#[test]
fn deep_nesting_formats_without_overflow() {
    let depth = MAX_DEPTH - 2;
    let parens = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
    check_formats(&parens, SourceKind::Expression);
    let calls = format!("{}1{}", "ABS(".repeat(depth), ")".repeat(depth));
    check_formats(&calls, SourceKind::Expression);
    let chain = vec!["1"; depth].join(" + ");
    check_formats(&chain, SourceKind::Expression);
    let mixed: String = (0..depth / 2).map(|i| format!("{i} * 2 + ")).collect();
    check_formats(&format!("{mixed}0"), SourceKind::Expression);
    let too_deep = format!(
        "{}1{}",
        "(".repeat(MAX_DEPTH + 1),
        ")".repeat(MAX_DEPTH + 1)
    );
    assert!(matches!(
        format(&too_deep, SourceKind::Expression, &FormatOptions::default()),
        Err(FormatError::Syntax(_))
    ));
}
