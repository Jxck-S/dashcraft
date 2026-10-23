//! Power Query M lexer, parser and printer (`pbi_m`).
//!
//! Invariants: never panics; error and token spans are valid byte ranges of
//! the source (on char boundaries);
//! `parse(print(parse(x))) == parse(x)` modulo spans, and printing is a fixed
//! point after one round (documents and expressions).
#![no_main]

use libfuzzer_sys::fuzz_target;
use pbi_m::{
    ParseError, Span, parse_document_with_comments, parse_expression, print_document, print_expr,
    tokenize,
};

fn check_span(src: &str, span: Span, what: &str) {
    assert!(
        span.start <= span.end && src.get(span.start..span.end).is_some(),
        "{what} span {span:?} outside the source or not on char boundaries ({} bytes)",
        src.len()
    );
}

fn check_error(src: &str, e: &ParseError) {
    check_span(src, e.span, "error");
    let _ = e.render(src);
}

fuzz_target!(|data: &[u8]| {
    let Ok(src) = std::str::from_utf8(data) else {
        return;
    };
    match tokenize(src) {
        Ok(lexed) => {
            for token in &lexed.tokens {
                check_span(src, token.span, "token");
            }
            for comment in &lexed.comments {
                check_span(src, comment.span, "comment");
            }
        }
        Err(e) => check_error(src, &e),
    }

    match parse_document_with_comments(src) {
        Ok(parsed) => {
            let printed = print_document(&parsed.document);
            let reparsed = match parse_document_with_comments(&printed) {
                Ok(r) => r.document,
                Err(e) => panic!(
                    "printed document does not parse: {}\nsource:  {src:?}\nprinted: {printed:?}",
                    e.render(&printed)
                ),
            };
            let mut a = parsed.document;
            let mut b = reparsed.clone();
            a.clear_spans();
            b.clear_spans();
            assert!(
                a == b,
                "document changed through the printer\nsource:  {src:?}\nprinted: {printed:?}\nbefore: {a:?}\nafter:  {b:?}"
            );
            assert_eq!(
                print_document(&reparsed),
                printed,
                "printing is not a fixed point; source: {src:?}"
            );
        }
        Err(e) => check_error(src, &e),
    }

    match parse_expression(src) {
        Ok(expr) => {
            let printed = print_expr(&expr);
            let reparsed = match parse_expression(&printed) {
                Ok(r) => r,
                Err(e) => panic!(
                    "printed expression does not parse: {}\nsource:  {src:?}\nprinted: {printed:?}",
                    e.render(&printed)
                ),
            };
            let mut a = expr;
            let mut b = reparsed.clone();
            a.clear_spans();
            b.clear_spans();
            assert!(
                a == b,
                "expression changed through the printer\nsource:  {src:?}\nprinted: {printed:?}\nbefore: {a:?}\nafter:  {b:?}"
            );
            assert_eq!(
                print_expr(&reparsed),
                printed,
                "printing is not a fixed point; source: {src:?}"
            );
        }
        Err(e) => check_error(src, &e),
    }
});
