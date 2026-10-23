//! Parser tests: fixture M sources, AST shapes, precedence, literals, documents and errors.

use std::path::PathBuf;

use pbi_m::ast::{
    BinaryOp, Document, Expr, ExprKind, ListItem, Literal, PrimitiveType, TableRowType, TryHandler,
    TypeKind, UnaryOp,
};
use pbi_m::{CommentKind, ParseError, line_col, parse_document, parse_document_with_comments};
use pretty_assertions::assert_eq;

fn data_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn read_data(name: &str) -> String {
    std::fs::read_to_string(data_dir().join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn parse(src: &str) -> Expr {
    match pbi_m::parse_expression(src) {
        Ok(e) => e,
        Err(e) => panic!("parse failed: {}\n{src}", e.render(src)),
    }
}

fn parse_err(src: &str) -> ParseError {
    match pbi_m::parse_expression(src) {
        Ok(e) => panic!("expected an error for {src:?}, got {e:?}"),
        Err(e) => e,
    }
}

/// Compact, fully parenthesized rendering of the tree for precedence assertions.
fn sx(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Literal(Literal::Number(n)) => n.raw.clone(),
        ExprKind::Literal(Literal::Text(t)) => format!("{t:?}"),
        ExprKind::Literal(Literal::Logical(b)) => b.to_string(),
        ExprKind::Literal(Literal::Null) => "null".into(),
        ExprKind::Literal(Literal::Verbatim(t)) => format!("#!{t:?}"),
        ExprKind::Identifier { name, inclusive } => {
            format!("{}{}", if *inclusive { "@" } else { "" }, name.text)
        }
        ExprKind::Intrinsic(i) => i.keyword().into(),
        ExprKind::SectionAccess { section, member } => format!("{}!{}", section.text, member.text),
        ExprKind::Parenthesized(inner) => format!("(paren {})", sx(inner)),
        ExprKind::Let { bindings, body } => {
            let b: Vec<String> = bindings
                .iter()
                .map(|b| format!("{}={}", b.name.text, sx(&b.value)))
                .collect();
            format!("(let [{}] {})", b.join(" "), sx(body))
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => format!(
            "(if {} {} {})",
            sx(condition),
            sx(then_branch),
            sx(else_branch)
        ),
        ExprKind::Each(body) => format!("(each {})", sx(body)),
        ExprKind::Function { params, body, .. } => {
            let p: Vec<&str> = params.iter().map(|p| p.name.text.as_str()).collect();
            format!("(fn [{}] {})", p.join(" "), sx(body))
        }
        ExprKind::Invoke { function, args } => {
            let a: Vec<String> = args.iter().map(sx).collect();
            format!("(call {} {})", sx(function), a.join(" "))
        }
        ExprKind::Record(fields) => {
            let f: Vec<String> = fields
                .iter()
                .map(|f| format!("{}={}", f.name.text, sx(&f.value)))
                .collect();
            format!("[{}]", f.join(" "))
        }
        ExprKind::List(items) => {
            let i: Vec<String> = items
                .iter()
                .map(|i| match i {
                    ListItem::Single(e) => sx(e),
                    ListItem::Range { start, end } => format!("{}..{}", sx(start), sx(end)),
                })
                .collect();
            format!("{{{}}}", i.join(" "))
        }
        ExprKind::FieldAccess {
            target,
            field,
            optional,
        } => format!(
            "(field {} {}{})",
            target.as_ref().map_or("_".to_owned(), |t| sx(t)),
            field.text,
            if *optional { "?" } else { "" }
        ),
        ExprKind::FieldProjection {
            target,
            fields,
            optional,
        } => {
            let f: Vec<&str> = fields.iter().map(|f| f.text.as_str()).collect();
            format!(
                "(project {} {}{})",
                target.as_ref().map_or("_".to_owned(), |t| sx(t)),
                f.join(" "),
                if *optional { "?" } else { "" }
            )
        }
        ExprKind::ItemAccess {
            target,
            selector,
            optional,
        } => format!(
            "(item {} {}{})",
            sx(target),
            sx(selector),
            if *optional { "?" } else { "" }
        ),
        ExprKind::Unary { op, operand } => format!("({} {})", op.symbol(), sx(operand)),
        ExprKind::Binary { op, lhs, rhs } => {
            format!("({} {} {})", op.symbol(), sx(lhs), sx(rhs))
        }
        ExprKind::As { expr, ty } => format!(
            "(as {} {}{})",
            sx(expr),
            if ty.nullable { "nullable " } else { "" },
            ty.primitive.name()
        ),
        ExprKind::Is { expr, ty } => format!(
            "(is {} {}{})",
            sx(expr),
            if ty.nullable { "nullable " } else { "" },
            ty.primitive.name()
        ),
        ExprKind::Type(t) => format!("(type {t})"),
        ExprKind::Try { expr, handler } => match handler {
            None => format!("(try {})", sx(expr)),
            Some(TryHandler::Otherwise(d)) => format!("(try {} otherwise {})", sx(expr), sx(d)),
            Some(TryHandler::Catch { param, body }) => format!(
                "(try {} catch {} {})",
                sx(expr),
                param.as_ref().map_or("()", |p| p.text.as_str()),
                sx(body)
            ),
        },
        ExprKind::Error(e) => format!("(error {})", sx(e)),
        ExprKind::NotImplemented => "...".into(),
    }
}

fn sxp(src: &str) -> String {
    sx(&parse(src))
}

fn let_binding<'a>(e: &'a Expr, name: &str) -> &'a Expr {
    let ExprKind::Let { bindings, .. } = &e.kind else {
        panic!("not a let: {e:?}")
    };
    &bindings
        .iter()
        .find(|b| b.name.text == name)
        .unwrap_or_else(|| panic!("no binding {name}"))
        .value
}

// ---------------------------------------------------------------------------------------------
// Fixture sources
// ---------------------------------------------------------------------------------------------

#[test]
fn all_test_data_files_parse() {
    let mut count = 0;
    for entry in std::fs::read_dir(data_dir()).unwrap() {
        let path = entry.unwrap().path();
        let src = std::fs::read_to_string(&path).unwrap();
        if let Err(e) = parse_document(&src) {
            panic!("{}: {}", path.display(), e.render(&src));
        }
        if path.extension().is_some_and(|x| x == "m") {
            parse(&src);
        }
        count += 1;
    }
    assert!(count >= 11, "expected the fixture files, found {count}");
}

#[test]
fn orders_query_shape() {
    let e = parse(&read_data("orders.m"));
    let ExprKind::Let { bindings, body } = &e.kind else {
        panic!("not a let")
    };
    let names: Vec<&str> = bindings.iter().map(|b| b.name.text.as_str()).collect();
    assert_eq!(
        names,
        [
            "Source",
            "Source CSV",
            "Promoted Headers",
            "Removed Rows Without Id",
            "Cleaned Text",
            "Removed Duplicates",
            "Changed Type",
            "Default Discount",
            "Added Line Amount",
            "Renamed Columns"
        ]
    );
    assert!(bindings[1].name.quoted && !bindings[0].name.quoted);
    assert_eq!(sx(body), "Renamed Columns");

    // Source{[Name = "..."]}[Content] is field access on an item access with a record selector.
    let csv = let_binding(&e, "Source CSV");
    let ExprKind::Invoke { function, args } = &csv.kind else {
        panic!("not an invocation")
    };
    assert_eq!(sx(function), "Csv.Document");
    let ExprKind::FieldAccess {
        target: Some(target),
        field,
        optional: false,
    } = &args[0].kind
    else {
        panic!("not a field access: {:?}", args[0])
    };
    assert_eq!(field.text, "Content");
    let ExprKind::ItemAccess {
        target: source,
        selector,
        optional: false,
    } = &target.kind
    else {
        panic!("not an item access")
    };
    assert_eq!(sx(source), "Source");
    assert!(
        matches!(&selector.kind, ExprKind::Record(f) if f.len() == 1 && f[0].name.text == "Name")
    );
    assert_eq!(
        sx(&args[1]),
        r#"[Delimiter="," Columns=10 Encoding=65001 QuoteStyle=QuoteStyle.Csv]"#
    );

    // `??` inside a call argument, compared after the call.
    assert_eq!(
        sx(let_binding(&e, "Removed Rows Without Id")),
        r#"(call Table.SelectRows Promoted Headers (each (<> (call Text.Trim (?? (field _ order_id) "")) "")))"#
    );
    assert_eq!(
        sx(let_binding(&e, "Default Discount")),
        r#"(call Table.TransformColumns Changed Type {{"discount" (each (?? _ 0)) (type number)}})"#
    );
    assert_eq!(
        sx(let_binding(&e, "Added Line Amount")),
        r#"(call Table.AddColumn Default Discount "line_amount" (each (* (* (field _ quantity) (field _ unit_price)) (paren (- 1 (field _ discount))))) (type number))"#
    );
}

#[test]
fn promotions_query_functions_and_identifiers() {
    let e = parse(&read_data("promotions.m"));
    // `??` binds looser than everything else; `and` combines the two comparisons.
    assert_eq!(
        sx(let_binding(&e, "Running Campaigns")),
        r#"(call Table.SelectRows Promoted Headers (each (and (= (call Text.Trim (?? (field _ Campaign Status) "")) "Running") (<> (call Text.Trim (?? (field _ Promo Code) "")) ""))))"#
    );

    // (raw as nullable text) as nullable text => let ... in ...
    let clean = let_binding(&e, "CleanChannel");
    let ExprKind::Function {
        params,
        return_type,
        body,
    } = &clean.kind
    else {
        panic!("not a function")
    };
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].name.text, "raw");
    assert!(!params[0].optional);
    let ty = params[0].ty.unwrap();
    assert!(ty.nullable);
    assert_eq!(ty.primitive, PrimitiveType::Text);
    let rt = return_type.unwrap();
    assert!(rt.nullable && rt.primitive == PrimitiveType::Text);
    // The backslash is an ordinary character in M text.
    assert_eq!(
        sx(body),
        r#"(let [t=(call Text.Trim (?? raw ""))] (if (or (= t "") (= t "\\")) null (call Text.Proper t)))"#
    );

    let sale_price = let_binding(&e, "SalePrice");
    let ExprKind::Function {
        params,
        return_type,
        ..
    } = &sale_price.kind
    else {
        panic!("not a function")
    };
    let p: Vec<(&str, bool, PrimitiveType)> = params
        .iter()
        .map(|p| {
            let t = p.ty.unwrap();
            (p.name.text.as_str(), t.nullable, t.primitive)
        })
        .collect();
    assert_eq!(
        p,
        [
            ("price", false, PrimitiveType::Number),
            ("percent", false, PrimitiveType::Number)
        ]
    );
    assert_eq!(return_type.unwrap().primitive, PrimitiveType::Number);

    // Generalized and quoted identifiers in field access.
    let added = sx(let_binding(&e, "Added Sale Price"));
    assert!(added.contains("(field _ List Price)"), "{added}");
    assert!(added.contains("(field _ Discount (%))"), "{added}");
}

#[test]
fn customers_nested_let_in_each() {
    let e = parse(&read_data("customers.m"));
    let cleaned = sx(let_binding(&e, "Cleaned Text"));
    assert!(
        cleaned.contains(
            r#"{"loyalty_tier" (each (let [t=(call Text.Trim (?? _ ""))] (if (= t "") "Unassigned" t))) (type text)}"#
        ),
        "{cleaned}"
    );
    assert!(
        cleaned.contains(r#"{"customer_id" fnCleanKey (type text)}"#),
        "{cleaned}"
    );
}

#[test]
fn shared_functions_parse_with_typed_parameters() {
    let e = parse(&read_data("fn_clean_key.m"));
    assert_eq!(sx(&e), "(fn [x] (call Text.Upper (call Text.Trim x)))");
    let e = parse(&read_data("fn_parse_names.m"));
    let ExprKind::Function { body, .. } = &e.kind else {
        panic!("not a function")
    };
    // `""""` is a text literal holding one double quote.
    assert_eq!(
        sx(let_binding(body, "Names")),
        r#"(call List.Transform (call Text.Split Inner ",") (each (call Text.Trim (call Text.Trim _) {"'" "\""})))"#
    );
}

#[test]
fn last_refreshed_hash_table_with_table_type() {
    let e = parse(&read_data("last_refreshed.m"));
    let source = let_binding(&e, "Source");
    let ExprKind::Invoke { function, args } = &source.kind else {
        panic!("not an invocation")
    };
    assert!(matches!(
        function.kind,
        ExprKind::Intrinsic(pbi_m::ast::Intrinsic::Table)
    ));
    let ExprKind::Type(ty) = &args[0].kind else {
        panic!("not a type")
    };
    let TypeKind::Table(TableRowType::Fields(row)) = &ty.kind else {
        panic!("not a table type: {ty:?}")
    };
    assert_eq!(row.fields.len(), 1);
    assert_eq!(row.fields[0].name.text, "Last Refreshed (UTC)");
    assert!(row.fields[0].name.quoted);
    assert_eq!(
        row.fields[0].ty.as_ref().map(|t| &t.kind),
        Some(&TypeKind::Primitive(PrimitiveType::DateTime))
    );
    assert_eq!(
        sx(&args[1]),
        "{{(call DateTimeZone.RemoveZone (call DateTimeZone.UtcNow ))}}"
    );
}

#[test]
fn parameter_expression_with_meta() {
    let e = parse(&read_data("storage_container_url.m"));
    assert_eq!(
        sx(&e),
        r#"(meta "https://acmeoutfittersdemo.blob.core.windows.net/sample-data" [IsParameterQuery=true Type="Text" IsParameterQueryRequired=true])"#
    );
}

#[test]
fn comments_are_kept_as_trivia() {
    let src = read_data("promotions.m");
    let parsed = parse_document_with_comments(&src).unwrap();
    assert_eq!(parsed.comments.len(), 1);
    let c = &parsed.comments[0];
    assert_eq!(c.kind, CommentKind::Line);
    assert!(c.text.starts_with("// The campaign export writes"));
    assert_eq!(c.span.slice(&src), c.text);
}

// ---------------------------------------------------------------------------------------------
// Precedence and grammar
// ---------------------------------------------------------------------------------------------

#[test]
fn operator_precedence() {
    let cases = [
        ("1 + 2 * 3", "(+ 1 (* 2 3))"),
        ("1 - 2 - 3", "(- (- 1 2) 3)"),
        ("8 / 4 / 2", "(/ (/ 8 4) 2)"),
        ("a & b + c", "(+ (& a b) c)"),
        ("a & b * c", "(& a (* b c))"),
        ("a ?? b or c", "(?? a (or b c))"),
        ("a or b ?? c", "(?? (or a b) c)"),
        ("a ?? b ?? c", "(?? (?? a b) c)"),
        ("a or b and c", "(or a (and b c))"),
        ("a and b = c", "(and a (= b c))"),
        ("a = b < c", "(= a (< b c))"),
        ("a <> b >= c + d", "(<> a (>= b (+ c d)))"),
        ("a < b + c", "(< a (+ b c))"),
        ("a * b meta c", "(* a (meta b c))"),
        ("a meta b meta c", "(meta (meta a b) c)"),
        ("-a meta b", "(meta (- a) b)"),
        ("not a = b", "(= (not a) b)"),
        ("not not a", "(not (not a))"),
        ("- - 1", "(- (- 1))"),
        ("-x{0}", "(- (item x 0))"),
        ("a = b as number", "(as (= a b) number)"),
        ("x as number is logical", "(is (as x number) logical)"),
        ("x is number and y", "(and (is x number) y)"),
        ("x as nullable text ?? y", "(?? (as x nullable text) y)"),
        ("[a] ?? \"\" = \"x\"", "(?? (field _ a) (= \"\" \"x\"))"),
        ("each [a] + 1", "(each (+ (field _ a) 1))"),
        ("each each _", "(each (each _))"),
        ("(x) => x + 1", "(fn [x] (+ x 1))"),
        ("(x) as number", "(as (paren x) number)"),
        (
            "if a then b else if c then d else e + 1",
            "(if a b (if c d (+ e 1)))",
        ),
        ("f(x){0}[a]?", "(field (item (call f x) 0) a?)"),
        ("f(x)(y)", "(call (call f x) y)"),
        ("x[[a], [b c]]?", "(project x a b c?)"),
        ("[[a]]", "(project _ a)"),
        ("[a][b]", "(field (field _ a) b)"),
        ("{1..3, 5}", "{1..3 5}"),
        ("@f(1)", "(call @f 1)"),
        ("Section1!Member", "Section1!Member"),
        ("type text", "(type text)"),
        ("type nullable text", "(type nullable text)"),
        ("not type text", "(not (type text))"),
        ("try a otherwise b + 1", "(try a otherwise (+ b 1))"),
        ("try try a otherwise b", "(try (try a otherwise b))"),
        ("try a catch (e) => e", "(try a catch e e)"),
        ("try a catch () => 1", "(try a catch () 1)"),
        ("error \"x\" & y", "(error (& \"x\" y))"),
        ("...", "..."),
        ("#infinity", "#infinity"),
        ("(optional) => optional", "(fn [optional] optional)"),
    ];
    for (src, expected) in cases {
        assert_eq!(sxp(src), expected, "source: {src}");
    }
}

#[test]
fn records_vs_implicit_field_access() {
    assert_eq!(sxp("[]"), "[]");
    assert_eq!(sxp("[a = 1]"), "[a=1]");
    assert_eq!(sxp("[a]"), "(field _ a)");
    assert_eq!(sxp("[a]?"), "(field _ a?)");
    assert_eq!(sxp("[a b = 1, c.d = 2]"), "[a b=1 c.d=2]");
    assert_eq!(sxp("[Order Ship Date]"), "(field _ Order Ship Date)");
    assert_eq!(sxp("[type]"), "(field _ type)");
    assert_eq!(sxp("[if = 1, then = 2]"), "[if=1 then=2]");
    assert_eq!(sxp("[1st Place]"), "(field _ 1st Place)");
    assert_eq!(sxp("[Column1.2]"), "(field _ Column1.2)");
    assert_eq!(sxp("[#\"a]b\"]"), "(field _ a]b)");
    // Blanks inside generalized identifiers are kept verbatim.
    assert_eq!(sxp("[a  b]"), "(field _ a  b)");
    // A new-line is not a blank: two separate words are not one name.
    assert!(pbi_m::parse_expression("[a\nb]").is_err());
}

#[test]
fn literals_and_escapes() {
    let text = |src: &str| match parse(src).kind {
        ExprKind::Literal(Literal::Text(t)) => t,
        other => panic!("not text: {other:?}"),
    };
    assert_eq!(text(r#""a""b""#), "a\"b");
    assert_eq!(text(r#""\""#), "\\");
    assert_eq!(text(r##""#(cr,lf)""##), "\r\n");
    assert_eq!(text(r##""#(cr)#(lf)#(tab)#(#)(""##), "\r\n\t#(");
    assert_eq!(text(r##""#(000D)#(0000000A)""##), "\r\n");
    assert_eq!(text(r##""# not an escape""##), "# not an escape");

    let number = |src: &str| match parse(src).kind {
        ExprKind::Literal(Literal::Number(n)) => (n.raw, n.value),
        other => panic!("not a number: {other:?}"),
    };
    assert_eq!(number("0xFF"), ("0xFF".to_owned(), 255.0));
    assert_eq!(number("1.5e3"), ("1.5e3".to_owned(), 1500.0));
    assert_eq!(number(".25"), (".25".to_owned(), 0.25));
    assert!(matches!(
        parse("-1").kind,
        ExprKind::Unary {
            op: UnaryOp::Minus,
            ..
        }
    ));
    assert!(matches!(
        parse("true").kind,
        ExprKind::Literal(Literal::Logical(true))
    ));
    assert!(matches!(
        parse("null").kind,
        ExprKind::Literal(Literal::Null)
    ));
    assert_eq!(sxp(r#"#!"x""y""#), r##"#!"x\"y""##);
}

#[test]
fn quoted_identifiers() {
    let e = parse(r#"let #"My Step" = 1, #"let" = 2, #"a""b" = 3 in #"My Step" + #"let""#);
    let ExprKind::Let { bindings, body } = &e.kind else {
        panic!()
    };
    let names: Vec<(&str, bool)> = bindings
        .iter()
        .map(|b| (b.name.text.as_str(), b.name.quoted))
        .collect();
    assert_eq!(names, [("My Step", true), ("let", true), ("a\"b", true)]);
    assert_eq!(sx(body), "(+ My Step let)");
}

#[test]
fn types() {
    let ty = |src: &str| match parse(src).kind {
        ExprKind::Type(t) => *t,
        other => panic!("not a type: {other:?}"),
    };
    assert_eq!(
        ty("type text").kind,
        TypeKind::Primitive(PrimitiveType::Text)
    );
    assert!(matches!(
        ty("type table").kind,
        TypeKind::Primitive(PrimitiveType::Table)
    ));
    assert!(matches!(
        ty("type function").kind,
        TypeKind::Primitive(PrimitiveType::Function)
    ));
    assert!(matches!(ty("type {number}").kind, TypeKind::List(_)));
    let TypeKind::Record(r) = ty("type [a = text, optional b, ...]").kind else {
        panic!()
    };
    assert!(r.open);
    assert_eq!(r.fields.len(), 2);
    assert!(!r.fields[0].optional && r.fields[1].optional && r.fields[1].ty.is_none());
    let TypeKind::Record(r) = ty("type [optional = text, optional optional.x]").kind else {
        panic!()
    };
    let f: Vec<(&str, bool)> = r
        .fields
        .iter()
        .map(|f| (f.name.text.as_str(), f.optional))
        .collect();
    assert_eq!(f, [("optional", false), ("optional.x", true)]);
    let TypeKind::Table(TableRowType::Fields(row)) =
        ty("type table [A = Int64.Type, B = nullable number]").kind
    else {
        panic!()
    };
    assert!(
        matches!(&row.fields[0].ty.as_ref().unwrap().kind, TypeKind::Expr(e) if sx(e) == "Int64.Type")
    );
    assert!(matches!(
        &row.fields[1].ty.as_ref().unwrap().kind,
        TypeKind::Nullable(inner) if inner.kind == TypeKind::Primitive(PrimitiveType::Number)
    ));
    assert!(matches!(
        ty("type table RowType").kind,
        TypeKind::Table(TableRowType::Expr(_))
    ));
    let TypeKind::Function(f) =
        ty("type function (x as text, optional y as nullable number) as logical").kind
    else {
        panic!()
    };
    assert_eq!(f.params.len(), 2);
    assert!(f.params[1].optional);
    assert_eq!(
        f.return_type.kind,
        TypeKind::Primitive(PrimitiveType::Logical)
    );
    // `type` must be followed by a primary type, not an arbitrary expression.
    assert!(pbi_m::parse_expression("type Int64.Type").is_err());
    // Table row types are never open.
    assert!(pbi_m::parse_expression("type table [a, ...]").is_err());
}

#[test]
fn hash_table_with_type_table() {
    assert_eq!(
        sxp(r#"#table(type table [#"A B" = text, C = number], {{"x", 1}, {"y", 2}})"#),
        r#"(call #table (type table [#"A B" = text, C = number]) {{"x" 1} {"y" 2}})"#
    );
    assert_eq!(
        sxp(r#"#table({"a", "b"}, {})"#),
        r#"(call #table {"a" "b"} {})"#
    );
}

#[test]
fn try_otherwise_and_meta() {
    let e = parse(r#"try Number.From("x") otherwise 0"#);
    assert!(matches!(
        &e.kind,
        ExprKind::Try {
            handler: Some(TryHandler::Otherwise(_)),
            ..
        }
    ));
    assert_eq!(sxp("x meta [a=1]"), "(meta x [a=1])");
    let e = parse("x meta [a=1]");
    assert!(matches!(
        e.kind,
        ExprKind::Binary {
            op: BinaryOp::Meta,
            ..
        }
    ));
}

#[test]
fn section_documents() {
    let src = read_data("hello_world.pq");
    let Document::Section(section) = parse_document(&src).unwrap() else {
        panic!("not a section document")
    };
    assert_eq!(section.name.text, "HelloWorld");
    assert_eq!(
        section.attributes.as_ref().map(sx).as_deref(),
        Some(r#"[Version="1.0.0" Tags={"a" 1 true null [Nested=0x10]}]"#)
    );
    let members: Vec<(&str, bool, bool)> = section
        .members
        .iter()
        .map(|m| (m.name.text.as_str(), m.shared, m.attributes.is_some()))
        .collect();
    assert_eq!(
        members,
        [
            ("HelloWorld.Contents", true, true),
            ("HelloWorld", false, false),
            ("HelloWorld.Publish", false, false),
            ("HelloWorldType", false, false),
            ("Quoted Member", true, false),
        ]
    );
    assert_eq!(sx(&section.members[4].value), "HelloWorld!HelloWorldType");
    // `type function (...) as table meta [...]`: meta applies to the whole type.
    assert!(matches!(
        &section.members[3].value.kind,
        ExprKind::Binary { op: BinaryOp::Meta, lhs, .. } if matches!(lhs.kind, ExprKind::Type(_))
    ));

    let doc = parse_document("section S; a = 1; shared b = a;").unwrap();
    let Document::Section(s) = doc else { panic!() };
    assert_eq!(s.members.len(), 2);
    // A record at the start of an expression document is not mistaken for attributes.
    assert!(matches!(
        parse_document("[a = 1][a]").unwrap(),
        Document::Expression(_)
    ));
    assert!(matches!(
        parse_document("[a = 1]").unwrap(),
        Document::Expression(_)
    ));
    // Members must be terminated by `;`.
    assert!(parse_document("section S; a = 1").is_err());
}

// ---------------------------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------------------------

#[test]
fn error_unterminated_text() {
    let src = "let\n    a = \"abc\nin\n    a";
    let err = parse_err(src);
    assert!(err.message.contains("unterminated text literal"), "{err}");
    assert_eq!(err.span.start, 12);
    assert_eq!(err.line_col(src), (2, 9));
    assert_eq!(err.render(src), "2:9: unterminated text literal");
}

#[test]
fn error_missing_in() {
    let src = "let\n    a = 1,\n    b = 2\n    b";
    let err = parse_err(src);
    assert_eq!(err.expected, ["`,`", "`in`"]);
    assert_eq!(err.message, "expected `,` or `in`, found identifier `b`");
    assert_eq!(err.line_col(src), (4, 5));

    let err = parse_err("let a = 1");
    assert_eq!(err.message, "expected `,` or `in`, found end of input");
    assert_eq!(err.span.start, 9);
}

#[test]
fn error_unbalanced_brackets() {
    let src = "f(1,\n  {2, 3)";
    let err = parse_err(src);
    assert_eq!(
        err.message,
        "expected `,`, `..` or `}` (to close `{` at line 2, column 3), found `)`"
    );
    assert_eq!(err.line_col(src), (2, 8));

    let err = parse_err("(1 + 2");
    assert!(
        err.message.contains("to close `(` at line 1, column 1"),
        "{err}"
    );
    assert!(err.message.ends_with("found end of input"), "{err}");

    let err = parse_err("[a = 1");
    assert_eq!(err.expected, ["`,`", "`]`"]);

    let err = parse_err("1 + 2)");
    assert_eq!(err.message, "expected end of input, found `)`");
    assert_eq!(err.span.start, 5);

    let err = parse_err("[a = 1}");
    assert_eq!(err.span.start, 6);
}

#[test]
fn error_misc_syntax() {
    let cases = [
        ("", "expected expression, found end of input"),
        ("1 +", "expected expression, found end of input"),
        ("if a then b", "expected `else`, found end of input"),
        (
            "let a = 1, a = 2 in a",
            "variable `a` is defined more than once",
        ),
        ("[a = 1, a = 2]", "field `a` is defined more than once"),
        (
            "(optional a, b) => a",
            "required parameter `b` cannot follow an optional parameter",
        ),
        (
            "x as Int64.Type",
            "expected primitive type, found identifier `Int64.Type`",
        ),
        (
            "x as number = 1",
            "`=` cannot follow an `as`/`is` type test without parentheses",
        ),
        (
            "1 + if a then b else c",
            "expected expression, found keyword `if`",
        ),
        ("#foo", "unknown keyword `#foo`"),
        ("a $ b", "unexpected character `$`"),
        ("\"#(xyz)\"", "invalid character escape sequence"),
        ("/* never closed", "unterminated block comment"),
        ("{1,}", "expected expression, found `}`"),
    ];
    for (src, message) in cases {
        let err = parse_err(src);
        assert_eq!(err.message, message, "source: {src:?}");
        assert!(err.span.end <= src.len(), "span out of range for {src:?}");
    }
}

#[test]
fn deep_nesting_is_an_error_not_a_stack_overflow() {
    let n = 100_000;
    for (open, close) in [
        ("(", ")"),
        ("{", "}"),
        ("[a=", "]"),
        ("f(", ")"),
        ("x{", "}"),
    ] {
        let src = format!("{}1{}", open.repeat(n), close.repeat(n));
        let err = parse_err(&src);
        assert!(err.message.contains("nested too deeply"), "{open}: {err}");
    }
    let types = format!("type {}number{}", "{".repeat(n), "}".repeat(n));
    assert!(parse_err(&types).message.contains("nested too deeply"));
    let nullables = format!("type {}text", "nullable ".repeat(n));
    assert!(parse_err(&nullables).message.contains("nested too deeply"));
    for prefix in ["-", "not ", "each ", "try ", "error ", "() => "] {
        let src = format!("{}1", prefix.repeat(n));
        let err = parse_err(&src);
        assert!(err.message.contains("nested too deeply"), "{prefix}: {err}");
    }
    // Long left-associative and postfix chains are bounded by the tree depth limit.
    let chain = format!("1{}", " + 1".repeat(n));
    assert!(parse_err(&chain).message.contains("nested too deeply"));
    let postfix = format!("x{}", "{0}".repeat(n));
    assert!(parse_err(&postfix).message.contains("nested too deeply"));
    let elif = format!("{}0", "if a then 1 else ".repeat(n));
    assert!(parse_err(&elif).message.contains("nested too deeply"));
}

#[test]
fn reasonable_depth_is_accepted() {
    // Nesting just within the limits parses (and the result can be printed and dropped).
    let depth = pbi_m::MAX_NESTING - 2;
    let src = format!("{}1{}", "(".repeat(depth), ")".repeat(depth));
    let e = parse(&src);
    assert_eq!(parse(&pbi_m::print_expr(&e)), e);

    let chain = format!("1{}", " & 1".repeat(500));
    let e = parse(&chain);
    let printed = pbi_m::print_expr(&e);
    assert_eq!(printed, chain);

    let elif = format!("{}0", "if a then 1 else ".repeat(400));
    let e = parse(&elif);
    assert_eq!(parse(&pbi_m::print_expr(&e)), e);
}

#[test]
fn never_panics_on_truncated_or_mangled_input() {
    let mut sources: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(data_dir()).unwrap() {
        sources.push(std::fs::read_to_string(entry.unwrap().path()).unwrap());
    }
    for src in &sources {
        for (i, _) in src.char_indices() {
            let prefix = &src[..i];
            let _ = parse_document(prefix);
            // Drop one character in the middle as a cheap mutation.
            let mut mangled = String::with_capacity(src.len());
            mangled.push_str(prefix);
            mangled.extend(src[i..].chars().skip(1));
            let _ = parse_document(&mangled);
        }
    }
}

#[test]
fn line_col_helper() {
    let src = "let\r\n  x = 1\r\nin x";
    assert_eq!(line_col(src, 0), (1, 1));
    assert_eq!(line_col(src, 7), (2, 3));
    assert_eq!(line_col(src, src.len()), (3, 5));
}
