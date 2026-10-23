//! Differential tests: every query is evaluated with the fast paths and with
//! the general path only, on randomized small models (case variants of text
//! keys, BLANKs, unmatched keys and blank rows, duplicate one-side keys,
//! single and bidirectional relationships, a snowflake chain, a
//! disconnected table), and the results (values, types, errors) must be
//! identical.

use std::fmt::Write as _;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Decimal128Array, Float64Array, Int64Array, StringArray,
    TimestampMicrosecondArray,
};
use arrow::record_batch::RecordBatch;
use pbi_engine::{Engine, EngineSnapshot};
use pbi_model::{Column, ColumnRef, CrossFilter, DataType, Measure, Model, Relationship, Table};

use crate::eval::EvalOptions;
use crate::eval::interp::{Interp, Val, parse};

/// Deterministic xorshift64* generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    fn pick<T: Clone>(&mut self, items: &[T]) -> T {
        items[self.below(items.len())].clone()
    }
}

struct Fixture {
    snapshot: Arc<EngineSnapshot>,
    model: Model,
}

fn table(name: &str, columns: &[(&str, DataType)], measures: &[(&str, &str)]) -> Table {
    let mut t = Table::new(name);
    for (c, dt) in columns {
        t.columns.push(Column::data(*c, dt.clone()));
    }
    for (m, e) in measures {
        t.measures.push(Measure::new(*m, *e));
    }
    t
}

fn rel(from: (&str, &str), to: (&str, &str), both: bool, active: bool) -> Relationship {
    let mut r = Relationship::new(
        format!("{}_{}", from.0, to.0),
        ColumnRef::new(from.0, from.1),
        ColumnRef::new(to.0, to.1),
    );
    if both {
        r.cross_filtering_behavior = CrossFilter::BothDirections;
    }
    r.is_active = active;
    r
}

fn text_col(rng: &mut Rng, rows: usize, pool: &[Option<&str>]) -> ArrayRef {
    Arc::new(StringArray::from(
        (0..rows).map(|_| rng.pick(pool)).collect::<Vec<_>>(),
    ))
}

fn int_col(rng: &mut Rng, rows: usize, pool: &[Option<i64>]) -> ArrayRef {
    Arc::new(Int64Array::from(
        (0..rows).map(|_| rng.pick(pool)).collect::<Vec<_>>(),
    ))
}

fn batch(columns: Vec<(&str, ArrayRef)>) -> RecordBatch {
    RecordBatch::try_from_iter(columns).unwrap()
}

fn random_fixture(seed: u64) -> Fixture {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let fact_measures = [
        ("S", "SUM(Fact[N])"),
        ("SX", "SUM(Fact[X])"),
        ("Cnt", "COUNTROWS(Fact)"),
        ("CntBare", "COUNTROWS()"),
        ("Chain", "[S]"),
        ("Chain2", "([Chain])"),
        ("DC", "DISTINCTCOUNT(Fact[Cat])"),
        ("Avg", "AVERAGE([N])"),
        ("Ratio", "DIVIDE([S], [Cnt])"),
        ("Calc", "CALCULATE(SUM(Fact[N]), Dim1[Color] = \"red\")"),
        ("Loop", "[Loop2]"),
        ("Loop2", "[Loop]"),
        (
            "Groups",
            "COUNTROWS(SUMMARIZECOLUMNS(Fact[Cat], Dim3[Zone], \"v\", [S]))",
        ),
    ];
    let model = Model {
        tables: vec![
            table(
                "Fact",
                &[
                    ("Id", DataType::Int64),
                    ("K1", DataType::String),
                    ("K2", DataType::Int64),
                    ("Cat", DataType::String),
                    ("N", DataType::Int64),
                    ("X", DataType::Double),
                    ("D", DataType::Decimal),
                    ("B", DataType::Boolean),
                    ("Dt", DataType::DateTime),
                ],
                &fact_measures,
            ),
            table(
                "Dim1",
                &[
                    ("K1", DataType::String),
                    ("Color", DataType::String),
                    ("Size", DataType::Int64),
                ],
                &[("DimRows", "COUNTROWS(Dim1)"), ("Qty", "SUM(Fact[N])")],
            ),
            table(
                "Dim2",
                &[
                    ("K2", DataType::Int64),
                    ("Region", DataType::String),
                    ("K3", DataType::String),
                ],
                &[],
            ),
            table(
                "Dim3",
                &[("K3", DataType::String), ("Zone", DataType::String)],
                &[],
            ),
            table(
                "Fact2",
                &[("K1", DataType::String), ("M", DataType::Int64)],
                &[],
            ),
            table("Other", &[("Z", DataType::String)], &[]),
        ],
        relationships: vec![
            rel(("Fact", "K1"), ("Dim1", "K1"), rng.chance(30), true),
            rel(("Fact", "K2"), ("Dim2", "K2"), rng.chance(30), true),
            rel(("Dim2", "K3"), ("Dim3", "K3"), rng.chance(30), true),
            rel(("Fact2", "K1"), ("Dim1", "K1"), rng.chance(50), true),
            rel(("Fact", "Cat"), ("Other", "Z"), false, false),
        ],
        ..Model::default()
    };

    let fact_rows = rng.below(60);
    let unmatched = rng.chance(60);
    let k1_pool: Vec<Option<&str>> = if unmatched {
        vec![
            Some("k0"),
            Some("K1"),
            Some("k2"),
            Some("K3"),
            Some("zz"),
            None,
        ]
    } else {
        vec![Some("k0"), Some("K1"), Some("k2"), Some("K3")]
    };
    let k2_pool: Vec<Option<i64>> = if unmatched {
        vec![Some(0), Some(1), Some(2), Some(99), None]
    } else {
        vec![Some(0), Some(1), Some(2)]
    };
    let ids: Vec<i64> = (0..fact_rows as i64).map(|i| i * 7 % 101).collect();
    let xs = [
        Some(0.1),
        Some(0.2),
        Some(0.3),
        Some(-0.0),
        Some(0.0),
        Some(1e16),
        Some(2.5),
        Some(-7.25),
        None,
    ];
    let decimals = [Some(12_345i128), Some(-5_000), Some(1), None];
    let dates = [
        Some(1_700_000_000_000_000i64),
        Some(1_700_086_400_000_000),
        Some(1_600_000_000_000_000),
        None,
    ];
    let fact = batch(vec![
        ("Id", Arc::new(Int64Array::from(ids)) as ArrayRef),
        ("K1", text_col(&mut rng, fact_rows, &k1_pool)),
        ("K2", int_col(&mut rng, fact_rows, &k2_pool)),
        (
            "Cat",
            text_col(
                &mut rng,
                fact_rows,
                &[
                    Some("a"),
                    Some("A"),
                    Some("b"),
                    Some("B"),
                    Some("c"),
                    Some(""),
                    None,
                ],
            ),
        ),
        (
            "N",
            int_col(
                &mut rng,
                fact_rows,
                &[Some(-3), Some(0), Some(1), Some(2), Some(5), Some(6), None],
            ),
        ),
        (
            "X",
            Arc::new(Float64Array::from(
                (0..fact_rows).map(|_| rng.pick(&xs)).collect::<Vec<_>>(),
            )),
        ),
        (
            "D",
            Arc::new(
                Decimal128Array::from(
                    (0..fact_rows)
                        .map(|_| rng.pick(&decimals))
                        .collect::<Vec<_>>(),
                )
                .with_precision_and_scale(19, 4)
                .unwrap(),
            ),
        ),
        (
            "B",
            Arc::new(BooleanArray::from(
                (0..fact_rows)
                    .map(|_| rng.pick(&[Some(true), Some(false), None]))
                    .collect::<Vec<_>>(),
            )),
        ),
        (
            "Dt",
            Arc::new(TimestampMicrosecondArray::from(
                (0..fact_rows).map(|_| rng.pick(&dates)).collect::<Vec<_>>(),
            )),
        ),
    ]);

    let mut dim1_keys: Vec<Option<&str>> = vec![Some("K0"), Some("k1"), Some("K2")];
    if rng.chance(70) {
        dim1_keys.push(Some("k3"));
    }
    if rng.chance(25) {
        dim1_keys.push(Some("k0"));
    }
    if rng.chance(20) {
        dim1_keys.push(None);
    }
    let d1 = dim1_keys.len();
    let dim1 = batch(vec![
        ("K1", Arc::new(StringArray::from(dim1_keys)) as ArrayRef),
        (
            "Color",
            text_col(
                &mut rng,
                d1,
                &[Some("red"), Some("Red"), Some("blue"), Some(""), None],
            ),
        ),
        (
            "Size",
            int_col(&mut rng, d1, &[Some(1), Some(2), Some(3), None]),
        ),
    ]);
    let dim2_keys: Vec<i64> = if rng.chance(80) {
        vec![0, 1, 2]
    } else {
        vec![0, 2]
    };
    let d2 = dim2_keys.len();
    let dim2 = batch(vec![
        ("K2", Arc::new(Int64Array::from(dim2_keys)) as ArrayRef),
        (
            "Region",
            text_col(&mut rng, d2, &[Some("n"), Some("N"), Some("s"), None]),
        ),
        (
            "K3",
            text_col(&mut rng, d2, &[Some("x"), Some("y"), Some("q"), None]),
        ),
    ]);
    let dim3 = batch(vec![
        (
            "K3",
            Arc::new(StringArray::from(vec![Some("x"), Some("Y"), Some("z")])) as ArrayRef,
        ),
        (
            "Zone",
            text_col(&mut rng, 3, &[Some("z1"), Some("z2"), None]),
        ),
    ]);
    let f2 = rng.below(8);
    let fact2 = batch(vec![
        (
            "K1",
            text_col(&mut rng, f2, &[Some("k0"), Some("k1"), Some("zz")]),
        ),
        ("M", int_col(&mut rng, f2, &[Some(1), Some(2), None])),
    ]);
    let other = batch(vec![(
        "Z",
        Arc::new(StringArray::from(vec![Some("a"), Some("b"), None])) as ArrayRef,
    )]);
    let mut engine = Engine::new(&model);
    engine
        .load_tables([
            ("Fact".to_owned(), fact),
            ("Dim1".to_owned(), dim1),
            ("Dim2".to_owned(), dim2),
            ("Dim3".to_owned(), dim3),
            ("Fact2".to_owned(), fact2),
            ("Other".to_owned(), other),
        ])
        .unwrap();
    Fixture {
        snapshot: engine.snapshot(),
        model,
    }
}

/// Result (or error) of `src` as text, and the number of fast-path hits.
fn run(fx: &Fixture, src: &str, fast: bool) -> (String, usize) {
    let opts = EvalOptions {
        general_only: !fast,
        ..EvalOptions::default()
    };
    std::thread::scope(|scope| {
        std::thread::Builder::new()
            .stack_size(64 * 1024 * 1024)
            .spawn_scoped(scope, || {
                let interp = Interp::with_options(&fx.snapshot, &fx.model, &opts);
                let text = match parse(src).and_then(|e| interp.eval(&opts.env(), &e)) {
                    Ok(Val::Scalar(v)) => format!("{v:?}"),
                    Ok(Val::Table(t)) => {
                        let mut out = String::new();
                        for c in t.columns() {
                            let _ = write!(out, "{} ", c.qualified_name());
                        }
                        for row in t.rows() {
                            let _ = write!(out, "\n{row:?}");
                        }
                        out
                    }
                    Err(e) => format!("ERROR {e}"),
                };
                (text, interp.fast_path_hits.get())
            })
            .unwrap()
            .join()
            .unwrap()
    })
}

/// Evaluates `src` both ways; returns the fast-path hits.
fn check(fx: &Fixture, seed: u64, src: &str) -> usize {
    let (fast, hits) = run(fx, src, true);
    let (general, general_hits) = run(fx, src, false);
    assert_eq!(general_hits, 0);
    assert_eq!(
        fast, general,
        "seed {seed}: fast and general paths differ for\n{src}"
    );
    hits
}

const GROUPS: &[&str] = &[
    "Fact[Cat]",
    "Fact[Cat], Fact[B]",
    "Fact[N]",
    "Dim1[Color]",
    "Dim1[Color], Fact[Cat]",
    "Dim2[Region], Dim1[Color]",
    "Dim3[Zone]",
    "Dim3[Zone], Dim1[Size], Fact[B]",
    "Fact[Cat], Dim1[Color], Fact[N]",
    "Other[Z]",
    "Fact2[M]",
    "Dim1[K1]",
    "Fact[Id]",
    "'fact'[dt], DIM2[k3]",
];

const EXPRS: &[&str] = &[
    "\"v\", SUM(Fact[N])",
    "\"v\", SUM(Fact[X]), \"w\", COUNTROWS(Fact)",
    "\"v\", [S], \"w\", [Chain2]",
    "\"v\", MIN(Fact[Cat]), \"w\", MAX(Fact[Dt]), \"x\", AVERAGE(Fact[X])",
    "\"v\", COUNT(Fact[N]), \"w\", COUNTA(Fact[Cat]), \"x\", COUNTBLANK(Fact[N])",
    "\"v\", DISTINCTCOUNT(Fact[Cat]), \"w\", DISTINCTCOUNTNOBLANK(Fact[Cat])",
    "\"v\", SUM(Fact[D]), \"w\", AVERAGE(Fact[D]), \"x\", MIN(Fact[B])",
    "\"v\", IGNORE(COUNTROWS(Fact)), \"w\", SUM(Fact[N])",
    "\"v\", IGNORE([Cnt])",
    "\"v\", [Ratio]",
    "\"v\", SUM(Dim1[Size])",
    "\"v\", [CntBare], \"w\", [Avg]",
    "\"v\", SUM(Fact[Cat])",
    "\"v\", [DC], \"w\", MAX(Fact[B])",
    "\"v\", [Calc], \"w\", [S]",
    "\"v\", SUM(Fact2[M])",
    "\"v\", [Loop]",
    "\"v\", SUMX(Fact, Fact[N] * 2)",
    "\"v\", COUNTROWS(Dim1), \"w\", [DimRows]",
    "\"v\", [Qty]",
];

const CONTEXTS: &[&str] = &[
    "",
    "Dim1[Color] = \"red\"",
    "Fact[Cat] IN {\"a\", BLANK()}",
    "Dim1[Color] IN {BLANK()}",
    "Fact[N] > 1",
    "FILTER(Fact, Fact[N] <> 2)",
    "Fact2[M] = 1",
    "Dim3[Zone] = \"z1\"",
    "USERELATIONSHIP(Fact[Cat], Other[Z])",
    "ALL(Dim1)",
    "KEEPFILTERS(Dim2[Region] = \"n\")",
    "TREATAS({\"red\", \"blue\"}, Dim1[Color])",
    "Dim1[Size] IN {1, 2}, Fact[B] = TRUE()",
    "FILTER(ALL(Dim2), Dim2[Region] <> \"s\")",
];

/// Filter arguments of `SUMMARIZECOLUMNS` itself (between the group columns
/// and the extension pairs).
const INNER_FILTERS: &[&str] = &["", "Fact[N] > 0", "TREATAS({\"a\"}, Fact[Cat])"];

fn summarize_query(group: &str, inner: &str, exprs: &str, context: &str) -> String {
    let inner = if inner.is_empty() {
        String::new()
    } else {
        format!("{inner}, ")
    };
    let sc = format!("SUMMARIZECOLUMNS({group}, {inner}{exprs})");
    if context.is_empty() {
        sc
    } else {
        format!("CALCULATETABLE({sc}, {context})")
    }
}

#[test]
fn summarizecolumns_fast_path_matches_the_general_path() {
    let mut hits = 0;
    let mut queries = 0;
    for seed in 0..24u64 {
        let fx = random_fixture(seed);
        let mut rng = Rng(seed + 1000);
        // Every group/expression pair once per model, in a random context.
        for group in GROUPS {
            for exprs in EXPRS {
                let inner = rng.pick(INNER_FILTERS);
                let context = rng.pick(CONTEXTS);
                hits += check(&fx, seed, &summarize_query(group, inner, exprs, context));
                queries += 1;
            }
        }
    }
    assert!(
        hits * 4 > queries,
        "the fast path ran for only {hits} of {queries} queries"
    );
}

#[test]
fn summarizecolumns_fast_path_in_every_context() {
    for seed in [3u64, 11, 17] {
        let fx = random_fixture(seed);
        for context in CONTEXTS {
            for group in [
                "Fact[Cat], Dim1[Color]",
                "Dim3[Zone], Fact[N]",
                "Dim1[Color]",
            ] {
                for exprs in [EXPRS[0], EXPRS[2], EXPRS[4], EXPRS[5], EXPRS[7]] {
                    check(&fx, seed, &summarize_query(group, "", exprs, context));
                }
            }
        }
    }
}

#[test]
fn fast_path_is_taken_for_plain_aggregations() {
    let fx = random_fixture(5);
    for src in [
        "SUMMARIZECOLUMNS(Fact[Cat], \"v\", SUM(Fact[N]))",
        "SUMMARIZECOLUMNS(Fact[Cat], Fact[Id], \"v\", [Chain2], \"w\", COUNTROWS(Fact))",
        "SUMMARIZECOLUMNS(Dim1[Color], Dim3[Zone], \"v\", [S])",
        "CALCULATETABLE(SUMMARIZECOLUMNS(Dim1[Color], \"v\", [Cnt]), Fact[Cat] = \"a\")",
        "SUMMARIZECOLUMNS(Dim2[Region], Dim1[Color], \"v\", AVERAGE(Fact[X]))",
    ] {
        assert_eq!(check(&fx, 5, src), 1, "{src}");
    }
    for src in [
        "SUMMARIZECOLUMNS(Fact[Cat], \"v\", [Ratio])",
        "SUMMARIZECOLUMNS(Other[Z], \"v\", SUM(Fact[N]))",
        "SUMMARIZECOLUMNS(Fact2[M], \"v\", SUM(Fact[N]))",
        "SUMMARIZECOLUMNS(Fact[Cat], \"v\", SUM(Fact[N]), \"w\", SUM(Dim1[Size]))",
        "SUMMARIZECOLUMNS(ROLLUPADDISSUBTOTAL(Fact[Cat], \"T\"), \"v\", SUM(Fact[N]))",
        "CALCULATETABLE(SUMMARIZECOLUMNS(Fact[Cat], \"v\", SUM(Fact[N])), USERELATIONSHIP(Fact[Cat], Other[Z]))",
    ] {
        assert_eq!(check(&fx, 5, src), 0, "{src}");
    }
}

#[test]
fn summarizecolumns_under_row_context_and_transition() {
    for seed in 0..24u64 {
        let fx = random_fixture(seed);
        for src in [
            "ADDCOLUMNS(VALUES(Dim1[Color]), \"n\", [Groups])",
            "ADDCOLUMNS(Dim2, \"n\", [Groups])",
            "SUMX(VALUES(Dim1[Color]), COUNTROWS(SUMMARIZECOLUMNS(Fact[Cat], \"v\", [S])))",
            "ADDCOLUMNS(VALUES(Dim3[Zone]), \"t\", CONCATENATEX(SUMMARIZECOLUMNS(Fact[Cat], \"v\", [Chain]), [v], \",\"))",
            "GENERATE(VALUES(Dim1[Size]), CALCULATETABLE(SUMMARIZECOLUMNS(Dim1[Color], \"v\", [Cnt])))",
        ] {
            check(&fx, seed, src);
        }
    }
}

#[test]
fn query_measures_and_functions_use_the_same_paths() {
    let queries = [
        "DEFINE MEASURE Fact[QS] = SUM(Fact[N]) MEASURE Fact[QC] = [QS] \
         EVALUATE SUMMARIZECOLUMNS(Fact[Cat], Dim1[Color], \"v\", [QC], \"w\", [QS])",
        "DEFINE FUNCTION SUM = (x) => 1 \
         EVALUATE SUMMARIZECOLUMNS(Fact[Cat], \"v\", SUM(Fact[N]))",
        "DEFINE MEASURE Fact[Rev] = SUMX(Fact, Fact[N] * Fact[X]) \
         EVALUATE SUMMARIZECOLUMNS(Dim2[Region], \"v\", [Rev], \"w\", [S])",
        "DEFINE VAR T = SUMMARIZECOLUMNS(Fact[B], \"v\", COUNTROWS(Fact)) \
         EVALUATE ROW(\"n\", SUMX(T, [v]))",
    ];
    for seed in 0..12u64 {
        let fx = random_fixture(seed);
        for q in queries {
            let eval = |general_only: bool| {
                let opts = EvalOptions {
                    general_only,
                    ..EvalOptions::default()
                };
                match crate::eval::query::evaluate_query_with(&fx.snapshot, &fx.model, q, &opts) {
                    Ok(tables) => tables
                        .iter()
                        .map(|t| format!("{:?}", t.rows()))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    Err(e) => format!("ERROR {e}"),
                }
            };
            assert_eq!(eval(false), eval(true), "seed {seed}: {q}");
        }
    }
}

const ITERATORS: &[&str] = &[
    "SUMX(Fact, Fact[N] * Fact[X])",
    "SUMX(Fact, [N] + 1)",
    "SUMX(Fact, -Fact[N] ^ 2)",
    "SUMX(Fact, Fact[D] * 2)",
    "SUMX(Fact, Fact[D] / 3)",
    "SUMX(Fact, Fact[X] / Fact[N])",
    "SUMX(Fact, Fact[N] - Fact[D])",
    "AVERAGEX(Fact, Fact[N] - 1.5)",
    "AVERAGEX(Fact, Fact[Dt])",
    "MINX(Fact, Fact[Dt] + 1)",
    "MAXX(Fact, Fact[Dt] - Fact[Dt])",
    "MINX(Fact, Fact[Cat])",
    "MAXX(Fact, Fact[B])",
    "COUNTX(Fact, Fact[N] * 0)",
    "COUNTAX(Fact, Fact[Cat])",
    "SUMX(Fact, Fact[Cat] + 1)",
    "SUMX(Fact, Fact[Cat])",
    "SUMX(Fact, +Fact[Cat])",
    "SUMX(Fact, Fact[B] * 2)",
    "SUMX(ALL(Dim1), Dim1[Size] - 1)",
    "SUMX(VALUES(Dim1), Dim1[Size])",
    "SUMX(FILTER(Fact, Fact[N] > 2), Fact[X] / Fact[N])",
    "SUMX(Fact, [Qty])",
    "SUMX(Fact, Fact[N] + Dim1[Size])",
    "SUMX(Fact, 1)",
    "SUMX(ADDCOLUMNS(Fact, \"q\", 1), Fact[N])",
    "SUMX(Fact, Fact[N] * 9223372036854775807)",
    "SUMX('fact', 'FACT'[n] * fact[x])",
    "AVERAGEX(Dim2, Dim2[K2] / 0)",
    "SUMX(Fact, Fact[X] + Fact[X] + Fact[X])",
];

#[test]
fn columnar_iterators_match_the_general_path() {
    let mut hits = 0;
    let mut queries = 0;
    for seed in 0..24u64 {
        let fx = random_fixture(seed);
        let mut rng = Rng(seed + 2000);
        for it in ITERATORS {
            let context = rng.pick(CONTEXTS);
            let src = if context.is_empty() {
                format!("ROW(\"v\", {it})")
            } else {
                format!("ROW(\"v\", CALCULATE({it}, {context}))")
            };
            hits += check(&fx, seed, &src);
            queries += 1;
        }
        // Iterators inside a per-group evaluation (general SUMMARIZECOLUMNS).
        hits += check(
            &fx,
            seed,
            "SUMMARIZECOLUMNS(Dim1[Color], \"v\", SUMX(Fact, Fact[N] * Fact[X]), \"w\", MAXX(Fact, Fact[Dt] + 0.5))",
        );
    }
    assert!(
        hits * 2 > queries,
        "fast iterators ran {hits} times for {queries} queries"
    );
}
