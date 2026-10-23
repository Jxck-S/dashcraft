//! DAX lexer, parser, printer and reference visitor (`pbi_dax::syntax`).
//!
//! The input is parsed as an expression, a query (`DEFINE`/`EVALUATE`) and a
//! user-defined function definition.
//!
//! Invariants (as in `crates/pbi-dax/tests/roundtrip.rs`): never panics;
//! spans lie inside the source and nest; `parse(print(parse(x))) ==
//! parse(x)` modulo spans for the compact and pretty printers; compact
//! printing is a fixed point.
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_dax::syntax::{
    Expr, ExprKind, ParseError, PrintOptions, Query, Span, for_each_expr, parse_expression,
    parse_function_definition, parse_query, pretty_expr, pretty_query, print_expr, print_query,
    reference_sites, referenced_tables, references, tokenize,
};

const WIDTHS: [usize; 2] = [80, 1];

fn check_span(src: &str, span: Span, what: &str) {
    assert!(
        span.start <= span.end && src.get(span.start..span.end).is_some(),
        "{what} span {span:?} outside the source or not on char boundaries ({} bytes): {src:?}",
        src.len()
    );
}

fn check_error(src: &str, e: &ParseError) {
    check_span(src, e.span, "error");
    let _ = e.render(src);
}

fn check_spans(src: &str, root: &Expr) {
    for_each_expr(root, |e| {
        check_span(src, e.span, "expression");
        if e.kind != ExprKind::Omitted {
            assert!(!e.span.is_empty(), "empty span for {e:?} in {src:?}");
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
            assert_eq!(e.span, lhs.span.to(rhs.span), "binary span in {src:?}");
        }
    });
    for (_, span) in reference_sites(root) {
        check_span(src, span, "reference");
    }
    let _ = references(root);
    let _ = referenced_tables(root);
}

fn check_expr(src: &str, expr: &Expr) {
    check_spans(src, expr);
    let printed = print_expr(expr);
    let reparsed = match parse_expression(&printed) {
        Ok(r) => r,
        Err(e) => panic!(
            "printed expression does not parse: {}\nsource:  {src:?}\nprinted: {printed:?}",
            e.render(&printed)
        ),
    };
    assert!(
        reparsed.eq_ignoring_spans(expr),
        "expression changed through the printer\nsource:  {src:?}\nprinted: {printed:?}"
    );
    assert_eq!(
        print_expr(&reparsed),
        printed,
        "printing is not a fixed point; source: {src:?}"
    );
    for max_width in WIDTHS {
        let opts = PrintOptions {
            max_width,
            ..PrintOptions::default()
        };
        let pretty = pretty_expr(expr, &opts);
        match parse_expression(&pretty) {
            Ok(r) => assert!(
                r.eq_ignoring_spans(expr),
                "expression changed through the pretty printer\nsource: {src:?}\npretty: {pretty:?}"
            ),
            Err(e) => panic!(
                "pretty expression does not parse: {}\nsource: {src:?}\npretty: {pretty:?}",
                e.render(&pretty)
            ),
        }
    }
}

fn check_query(src: &str, query: &Query) {
    for e in query.expressions() {
        check_spans(src, e);
    }
    let printed = print_query(query);
    let reparsed = match parse_query(&printed) {
        Ok(r) => r,
        Err(e) => panic!(
            "printed query does not parse: {}\nsource:  {src:?}\nprinted: {printed:?}",
            e.render(&printed)
        ),
    };
    assert!(
        reparsed.eq_ignoring_spans(query),
        "query changed through the printer\nsource:  {src:?}\nprinted: {printed:?}"
    );
    assert_eq!(
        print_query(&reparsed),
        printed,
        "printing is not a fixed point; source: {src:?}"
    );
    for max_width in WIDTHS {
        let opts = PrintOptions {
            max_width,
            ..PrintOptions::default()
        };
        let pretty = pretty_query(query, &opts);
        match parse_query(&pretty) {
            Ok(r) => assert!(
                r.eq_ignoring_spans(query),
                "query changed through the pretty printer\nsource: {src:?}\npretty: {pretty:?}"
            ),
            Err(e) => panic!(
                "pretty query does not parse: {}\nsource: {src:?}\npretty: {pretty:?}",
                e.render(&pretty)
            ),
        }
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    for token in tokenize(src) {
        check_span(src, token.span, "token");
    }
    match parse_expression(src) {
        Ok(expr) => check_expr(src, &expr),
        Err(e) => check_error(src, &e),
    }
    match parse_query(src) {
        Ok(query) => check_query(src, &query),
        Err(e) => check_error(src, &e),
    }
    match parse_function_definition(src) {
        Ok(def) => {
            check_span(src, def.span, "function");
            check_spans(src, &def.body);
        }
        Err(e) => check_error(src, &e),
    }
});
