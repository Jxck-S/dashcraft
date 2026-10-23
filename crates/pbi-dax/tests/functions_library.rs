#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Math, statistical-distribution, date and bit functions: accuracy against
//! reference values, invalid arguments, BLANK handling, deterministic
//! `RAND`, and a never-panic sweep over every function of the library wave
//! with random argument shapes.

mod common;

use common::data::{batch, datetimes, ints};
use pbi_dax::{DaxError, compute_calculated_column, evaluate_scalar};
use pbi_engine::{Engine, FilterContext, Value};
use pbi_model::{Column, DataType, Model, Table};

fn model() -> Model {
    let mut t = Table::new("T");
    t.columns = vec![
        Column::data("N", DataType::Int64),
        Column::data("Day", DataType::DateTime),
        Column::calculated("R", DataType::Double, "RAND()"),
    ];
    Model {
        tables: vec![t],
        ..Model::default()
    }
}

struct Fx {
    engine: Engine,
    model: Model,
}

fn fx() -> Fx {
    let model = model();
    let mut engine = Engine::new(&model);
    engine
        .load_table(
            "T",
            batch(vec![
                ("N", ints(&[Some(1), Some(2), Some(4), Some(8), None])),
                (
                    "Day",
                    datetimes(&[
                        Some((2024, 12, 25, 0, 0, 0)),
                        Some((2024, 12, 26, 0, 0, 0)),
                        Some((2025, 1, 1, 0, 0, 0)),
                        None,
                        Some((2024, 12, 28, 0, 0, 0)),
                    ]),
                ),
            ]),
        )
        .unwrap();
    let r = compute_calculated_column(&engine.snapshot(), &model, "T", "R").unwrap();
    engine.add_column("T", "R", r).unwrap();
    Fx { engine, model }
}

impl Fx {
    fn eval(&self, src: &str) -> Result<Value, DaxError> {
        evaluate_scalar(
            &self.engine.snapshot(),
            &self.model,
            src,
            &FilterContext::new(),
        )
    }

    fn value(&self, src: &str) -> Value {
        self.eval(src).unwrap_or_else(|e| panic!("{src}: {e}"))
    }

    fn number(&self, src: &str) -> f64 {
        match self.value(src) {
            Value::Double(x) => x,
            Value::Integer(n) => n as f64,
            other => panic!("{src}: expected a number, got {other:?}"),
        }
    }
}

/// Reference values from an independent implementation (SciPy's
/// distributions), spanning far tails, large shape parameters and the
/// Excel documentation examples.
const REFERENCE: &[(&str, f64)] = &[
    ("NORM.S.DIST(-37.5, TRUE)", 4.605353009581956e-308),
    ("NORM.S.DIST(-37.5, FALSE)", 1.7282337322841054e-306),
    ("NORM.S.DIST(-10, TRUE)", 7.619853024160474e-24),
    ("NORM.S.DIST(-10, FALSE)", 7.69459862670642e-23),
    ("NORM.S.DIST(-5, TRUE)", 2.8665157187919344e-07),
    ("NORM.S.DIST(-5, FALSE)", 1.4867195147342979e-06),
    ("NORM.S.DIST(-1.96, TRUE)", 0.024997895148220435),
    ("NORM.S.DIST(-1.96, FALSE)", 0.058440944333451476),
    ("NORM.S.DIST(-0.5, TRUE)", 0.3085375387259869),
    ("NORM.S.DIST(-0.5, FALSE)", 0.3520653267642995),
    ("NORM.S.DIST(0.3, TRUE)", 0.6179114221889526),
    ("NORM.S.DIST(0.3, FALSE)", 0.38138781546052414),
    ("NORM.S.DIST(1.96, TRUE)", 0.9750021048517795),
    ("NORM.S.DIST(1.96, FALSE)", 0.058440944333451476),
    ("NORM.S.DIST(5, TRUE)", 0.9999997133484281),
    ("NORM.S.DIST(5, FALSE)", 1.4867195147342979e-06),
    ("NORM.S.DIST(8.2, TRUE)", 0.9999999999999999),
    ("NORM.S.DIST(8.2, FALSE)", 9.99837874849718e-16),
    ("NORM.DIST(42, 40, 1.5, TRUE)", 0.9087887802741321),
    ("NORM.DIST(42, 40, 1.5, FALSE)", 0.10934004978399577),
    ("NORM.S.INV(1e-300)", -37.0470962993612),
    ("NORM.S.INV(1e-10)", -6.361340902404056),
    ("NORM.S.INV(0.001)", -3.090232306167813),
    ("NORM.S.INV(0.025)", -1.9599639845400545),
    ("NORM.S.INV(0.3)", -0.5244005127080409),
    ("NORM.S.INV(0.7)", 0.5244005127080407),
    ("NORM.S.INV(0.975)", 1.959963984540054),
    ("NORM.S.INV(0.999999)", 4.753424308817087),
    ("NORM.INV(0.908789, 40, 1.5)", 42.00000200956616),
    ("EXPON.DIST(0.2, 10, TRUE)", 0.8646647167633873),
    ("EXPON.DIST(0.2, 10, FALSE)", 1.353352832366127),
    ("EXPON.DIST(1e-9, 3, TRUE)", 2.9999999955000005e-09),
    ("POISSON.DIST(2, 5, TRUE)", 0.12465201948308109),
    ("POISSON.DIST(2, 5, FALSE)", 0.08422433748856832),
    ("POISSON.DIST(0, 0.5, TRUE)", 0.6065306597126334),
    ("POISSON.DIST(0, 0.5, FALSE)", 0.6065306597126334),
    ("POISSON.DIST(10, 3.5, TRUE)", 0.998980605562383),
    ("POISSON.DIST(10, 3.5, FALSE)", 0.002295549827015358),
    ("POISSON.DIST(150, 120.25, TRUE)", 0.9961871251782346),
    ("POISSON.DIST(150, 120.25, FALSE)", 0.0010763608601901406),
    ("POISSON.DIST(3, 0.001, TRUE)", 0.9999999999999584),
    ("POISSON.DIST(3, 0.001, FALSE)", 1.665000833055625e-10),
    ("BETA.DIST(2, 8, 10, TRUE, 1, 3)", 0.6854705810546875),
    ("BETA.DIST(2, 8, 10, FALSE, 1, 3)", 1.4837646484375002),
    ("BETA.DIST(0.3, 0.5, 0.5, TRUE, 0, 1)", 0.36901011956554536),
    ("BETA.DIST(0.3, 0.5, 0.5, FALSE, 0, 1)", 0.6946091180428566),
    ("BETA.DIST(0.01, 2.5, 40, TRUE, 0, 1)", 0.02419314394161421),
    ("BETA.DIST(0.01, 2.5, 40, FALSE, 0, 1)", 5.386570771950463),
    ("BETA.DIST(0.999, 30, 1.5, TRUE, 0, 1)", 0.9961107316148197),
    ("BETA.DIST(0.999, 30, 1.5, FALSE, 0, 1)", 5.766405882268388),
    ("BETA.DIST(0.45, 250, 300, TRUE, 0, 1)", 0.41624180485621237),
    ("BETA.DIST(0.45, 250, 300, FALSE, 0, 1)", 18.3874065977072),
    ("BETA.INV(0.685470581, 8, 10, 1, 3)", 1.9999999999631428),
    ("BETA.INV(0.05, 0.5, 0.5, 0, 1)", 0.0061558297024311365),
    ("BETA.INV(0.999, 2.5, 40, 0, 1)", 0.2226372995935286),
    ("BETA.INV(1e-08, 30, 1.5, 0, 1)", 0.5149705914168327),
    ("BETA.INV(0.5, 250, 300, 0, 1)", 0.454490318276659),
    ("CHISQ.DIST(0.5, 1, TRUE)", 0.5204998778130466),
    ("CHISQ.DIST(0.5, 1, FALSE)", 0.43939128946772243),
    ("CHISQ.DIST.RT(0.5, 1)", 0.47950012218695337),
    ("CHISQ.DIST(2, 3, TRUE)", 0.42759329552912023),
    ("CHISQ.DIST(2, 3, FALSE)", 0.2075537487102974),
    ("CHISQ.DIST.RT(2, 3)", 0.5724067044708798),
    ("CHISQ.DIST(18.307, 10, TRUE)", 0.949999410908602),
    ("CHISQ.DIST(18.307, 10, FALSE)", 0.015480784533096712),
    ("CHISQ.DIST.RT(18.307, 10)", 0.050000589091398075),
    ("CHISQ.DIST(1e-06, 4, TRUE)", 1.249999583333414e-13),
    ("CHISQ.DIST(1e-06, 4, FALSE)", 2.499998750000312e-07),
    ("CHISQ.DIST.RT(1e-06, 4)", 0.999999999999875),
    ("CHISQ.DIST(300, 250, TRUE)", 0.9834817268421313),
    ("CHISQ.DIST(300, 250, FALSE)", 0.0016302266818550082),
    ("CHISQ.DIST.RT(300, 250)", 0.016518273157868715),
    ("CHISQ.DIST(0.01, 1, TRUE)", 0.07965567455405799),
    ("CHISQ.DIST(0.01, 1, FALSE)", 3.9695254747701174),
    ("CHISQ.DIST.RT(0.01, 1)", 0.920344325445942),
    ("CHISQ.INV(0.93, 1)", 3.283020286759539),
    ("CHISQ.INV.RT(0.93, 1)", 0.007716715544951964),
    ("CHISQ.INV(0.6, 2)", 1.83258146374831),
    ("CHISQ.INV.RT(0.6, 2)", 1.0216512475319817),
    ("CHISQ.INV(1e-09, 3)", 2.417989100358596e-06),
    ("CHISQ.INV.RT(1e-09, 3)", 44.841275330562404),
    ("CHISQ.INV(0.5, 1000)", 999.333412403381),
    ("CHISQ.INV.RT(0.5, 1000)", 999.333412403381),
    ("CHISQ.INV(0.999999, 7)", 40.52183123411472),
    ("CHISQ.INV.RT(0.999999, 7)", 0.07852360099102075),
    ("T.DIST(60, 1, TRUE)", 0.9946953263673767),
    ("T.DIST(60, 1, FALSE)", 8.839485870141359e-05),
    ("T.DIST.RT(60, 1)", 0.005304673632623272),
    ("T.DIST.2T(60, 1)", 0.010609347265246543),
    ("T.DIST(8, 3, TRUE)", 0.9979617112061072),
    ("T.DIST(8, 3, FALSE)", 0.0007369065209469264),
    ("T.DIST.RT(8, 3)", 0.002038288793892733),
    ("T.DIST.2T(8, 3)", 0.004076577587785466),
    ("T.DIST(1.959999998, 60, TRUE)", 0.9726775350120396),
    ("T.DIST(1.959999998, 60, FALSE)", 0.059847906435580515),
    ("T.DIST.RT(1.959999998, 60)", 0.027322464987960433),
    ("T.DIST.2T(1.959999998, 60)", 0.05464492997592087),
    ("T.DIST(-2.5, 4, TRUE)", 0.03338327240599407),
    ("T.DIST(-2.5, 4, FALSE)", 0.03567562436955665),
    ("T.DIST.RT(-2.5, 4)", 0.966616727594006),
    ("T.DIST(0.001, 30, TRUE)", 0.5003956321167575),
    ("T.DIST(0.001, 30, FALSE)", 0.3956319804841909),
    ("T.DIST.RT(0.001, 30)", 0.49960436788324253),
    ("T.DIST.2T(0.001, 30)", 0.9992087357664851),
    ("T.DIST(35, 2, TRUE)", 0.9995923358474286),
    ("T.DIST(35, 2, FALSE)", 2.3266612450689455e-05),
    ("T.DIST.RT(35, 2)", 0.00040766415257147923),
    ("T.DIST.2T(35, 2)", 0.0008153283051429585),
    ("T.DIST(-1000.0, 5, TRUE)", 9.490065565989857e-15),
    ("T.DIST(-1000.0, 5, FALSE)", 4.745012447238592e-17),
    ("T.DIST.RT(-1000.0, 5)", 0.9999999999999906),
    ("T.INV(0.75, 2)", 0.8164965809277261),
    ("T.INV(0.025, 10)", -2.228138851986275),
    ("T.INV(1e-12, 3)", -10331.108244292487),
    ("T.INV(0.9999, 1)", 3183.098757118502),
    ("T.INV(0.6, 1000)", 0.2534145158394989),
    ("T.INV.2T(0.05, 10)", 2.228138851986275),
    ("T.INV.2T(0.546449, 60)", 0.6065330758257552),
    ("T.INV.2T(1e-10, 2)", 99999.99999249999),
    ("T.INV.2T(0.999, 5)", 0.0013171532190937648),
    ("CONFIDENCE.NORM(0.05, 2.5, 50)", 0.6929519121748391),
    ("CONFIDENCE.T(0.05, 1, 50)", 0.2841968554957299),
];

#[test]
fn distributions_match_reference_values_to_1e_12() {
    let f = fx();
    for (src, want) in REFERENCE {
        let got = f.number(src);
        let err = if *want == 0.0 {
            got.abs()
        } else {
            ((got - want) / want).abs()
        };
        assert!(
            err <= 1e-12,
            "{src}: {got:e} vs {want:e} (relative error {err:e})"
        );
    }
}

/// Evaluates `src` and returns the function named in the argument error.
fn argument_error(f: &Fx, src: &str) -> String {
    match f.eval(src) {
        Err(DaxError::Argument { function, message }) => {
            assert!(!message.is_empty(), "{src}");
            function
        }
        other => panic!("{src}: expected an argument error, got {other:?}"),
    }
}

#[test]
fn invalid_arguments_are_errors_naming_the_function() {
    let f = fx();
    let cases = [
        ("LN(0)", "LN"),
        ("LN(-1)", "LN"),
        ("LN(BLANK())", "LN"),
        ("LOG(-1)", "LOG"),
        ("LOG(10, 1)", "LOG"),
        ("LOG(10, -2)", "LOG"),
        ("LOG10(0)", "LOG10"),
        ("SQRTPI(-1)", "SQRTPI"),
        ("ASIN(2)", "ASIN"),
        ("ACOS(-1.5)", "ACOS"),
        ("ACOSH(0.5)", "ACOSH"),
        ("ATANH(1)", "ATANH"),
        ("ACOTH(0.5)", "ACOTH"),
        ("COT(0)", "COT"),
        ("COTH(0)", "COTH"),
        ("EXP(1000)", "EXP"),
        ("SINH(1000)", "SINH"),
        ("COSH(-1000)", "COSH"),
        ("LN(1 / 0)", "LN"),
        ("SIN(0 / 0)", "SIN"),
        ("FACT(-1)", "FACT"),
        ("FACT(171)", "FACT"),
        ("COMBIN(2, 3)", "COMBIN"),
        ("COMBIN(-1, 0)", "COMBIN"),
        ("COMBIN(5000, 2500)", "COMBIN"),
        ("COMBINA(2, 3)", "COMBINA"),
        ("PERMUT(3, 4)", "PERMUT"),
        ("PERMUT(1000, 500)", "PERMUT"),
        ("GCD(-1, 2)", "GCD"),
        ("GCD(POWER(2, 53), 1)", "GCD"),
        ("LCM(-2, 3)", "LCM"),
        ("LCM(4503599627370496, 3)", "LCM"),
        ("QUOTIENT(1, 0)", "QUOTIENT"),
        ("MROUND(5, -2)", "MROUND"),
        ("CEILING(2.5, -1)", "CEILING"),
        ("FLOOR(2.5, -1)", "FLOOR"),
        ("FLOOR(2.5, 0)", "FLOOR"),
        ("RANDBETWEEN(5, 1)", "RANDBETWEEN"),
        ("BITAND(1E19, 1)", "BITAND"),
        ("PERCENTILEX.EXC(T, T[N], 0.99)", "PERCENTILEX.EXC"),
        ("PERCENTILE.INC(T[N], 1.5)", "PERCENTILE.INC"),
        ("GEOMEANX({-1, 2}, [Value])", "GEOMEANX"),
        ("GEOMEANX({0, 2}, [Value])", "GEOMEANX"),
        ("NORM.DIST(1, 0, 0, TRUE)", "NORM.DIST"),
        ("NORM.INV(0, 0, 1)", "NORM.INV"),
        ("NORM.INV(1, 0, 1)", "NORM.INV"),
        ("NORM.S.INV(1.5)", "NORM.S.INV"),
        ("EXPON.DIST(-1, 1, TRUE)", "EXPON.DIST"),
        ("EXPON.DIST(1, 0, TRUE)", "EXPON.DIST"),
        ("POISSON.DIST(-1, 1, TRUE)", "POISSON.DIST"),
        ("POISSON.DIST(1, -1, TRUE)", "POISSON.DIST"),
        ("BETA.DIST(2, 1, 1, TRUE)", "BETA.DIST"),
        ("BETA.DIST(0.5, 0, 1, TRUE)", "BETA.DIST"),
        ("BETA.DIST(0.5, 1, 1, TRUE, 1, 1)", "BETA.DIST"),
        ("BETA.DIST(0, 0.5, 1, FALSE)", "BETA.DIST"),
        ("BETA.INV(0, 1, 1)", "BETA.INV"),
        ("BETA.INV(1.5, 1, 1)", "BETA.INV"),
        ("CHISQ.DIST(-1, 2, TRUE)", "CHISQ.DIST"),
        ("CHISQ.DIST(1, 0, TRUE)", "CHISQ.DIST"),
        ("CHISQ.DIST(1, 2E10, TRUE)", "CHISQ.DIST"),
        ("CHISQ.DIST(0, 1, FALSE)", "CHISQ.DIST"),
        ("CHISQ.INV(1, 2)", "CHISQ.INV"),
        ("CHISQ.INV(-0.1, 2)", "CHISQ.INV"),
        ("CHISQ.INV.RT(0, 2)", "CHISQ.INV.RT"),
        ("T.DIST(1, 0.5, TRUE)", "T.DIST"),
        ("T.DIST.2T(-1, 3)", "T.DIST.2T"),
        ("T.INV(0, 3)", "T.INV"),
        ("T.INV(1, 3)", "T.INV"),
        ("T.INV.2T(0, 3)", "T.INV.2T"),
        ("T.INV.2T(1.5, 3)", "T.INV.2T"),
        ("CONFIDENCE.NORM(0, 1, 10)", "CONFIDENCE.NORM"),
        ("CONFIDENCE.NORM(0.05, 0, 10)", "CONFIDENCE.NORM"),
        ("CONFIDENCE.NORM(0.05, 1, 0.5)", "CONFIDENCE.NORM"),
        ("CONFIDENCE.T(0.05, 1, 1)", "CONFIDENCE.T"),
        (
            "YEARFRAC(DATE(2024, 1, 1), DATE(2024, 2, 1), 5)",
            "YEARFRAC",
        ),
        (
            "YEARFRAC(DATE(2024, 1, 1), DATE(2024, 2, 1), -1)",
            "YEARFRAC",
        ),
        (
            "NETWORKDAYS(DATE(2024, 1, 1), DATE(2024, 2, 1), 9)",
            "NETWORKDAYS",
        ),
        (
            "NETWORKDAYS(DATE(2024, 1, 1), DATE(2024, 2, 1), \"1111111\")",
            "NETWORKDAYS",
        ),
        (
            "NETWORKDAYS(DATE(2024, 1, 1), DATE(2024, 2, 1), 1, {(1, 2)})",
            "NETWORKDAYS",
        ),
        // Wrong argument counts.
        ("LN()", "LN"),
        ("PI(1)", "PI"),
        ("LOG(1, 2, 3)", "LOG"),
        ("NORM.DIST(1, 2)", "NORM.DIST"),
        ("BITAND(1)", "BITAND"),
        ("GCD(1, 2, 3)", "GCD"),
        ("ISAFTER(1)", "ISAFTER"),
        ("ISAFTER(1, 2, ASC, 3)", "ISAFTER"),
    ];
    for (src, function) in cases {
        assert_eq!(argument_error(&f, src), function, "{src}");
    }
    // Text that is not a number is a type error, like the operators.
    assert!(matches!(f.eval("LN(\"abc\")"), Err(DaxError::Type { .. })));
    assert!(matches!(
        f.eval("ERROR(\"Custom failure\")"),
        Err(DaxError::Evaluation { message }) if message == "Custom failure"
    ));
    assert!(matches!(
        f.eval("IF.EAGER(TRUE(), 1, ERROR(\"eager\"))"),
        Err(DaxError::Evaluation { .. })
    ));
    assert_eq!(f.value("IF(TRUE(), 1, ERROR(\"lazy\"))"), Value::Integer(1));
}

#[test]
fn blank_arguments_follow_the_numeric_conventions() {
    let f = fx();
    let cases = [
        ("SIN(BLANK())", Value::Double(0.0)),
        ("COS(BLANK())", Value::Double(1.0)),
        ("FACT(BLANK())", Value::Double(1.0)),
        ("EVEN(BLANK())", Value::Integer(0)),
        ("ODD(BLANK())", Value::Integer(1)),
        ("BITAND(BLANK(), 5)", Value::Integer(0)),
        ("LOG(100, BLANK())", Value::Double(2.0)),
        ("NORM.S.DIST(BLANK(), TRUE)", Value::Double(0.5)),
        ("CEILING(BLANK(), 1)", Value::Blank),
        ("FLOOR(BLANK(), 1)", Value::Blank),
        ("ISO.CEILING(BLANK())", Value::Blank),
        ("MROUND(BLANK(), 3)", Value::Blank),
        ("CEILING(2.5, BLANK())", Value::Double(0.0)),
        ("YEARFRAC(BLANK(), BLANK())", Value::Double(0.0)),
        ("NETWORKDAYS(BLANK(), BLANK())", Value::Blank),
        (
            "GEOMEAN(T[N])",
            Value::Double(8f64.ln().mul_add(0.5, 0.0).exp()),
        ),
        ("CALCULATE(GEOMEAN(T[N]), T[N] > 100)", Value::Blank),
        ("CALCULATE(MAXA(T[N]), T[N] > 100)", Value::Blank),
        ("CALCULATE(MAXA(T[N]), ISBLANK(T[N]))", Value::Integer(0)),
        ("CALCULATE(AVERAGEA(T[N]), T[N] > 100)", Value::Blank),
        (
            "CALCULATE(AVERAGEA(T[N]), ISBLANK(T[N]))",
            Value::Double(0.0),
        ),
        ("ISAFTER(BLANK(), 0)", Value::Boolean(false)),
        ("ISONORAFTER(BLANK(), 0)", Value::Boolean(true)),
    ];
    for (src, want) in cases {
        let got = f.value(src);
        match (&got, &want) {
            (Value::Double(a), Value::Double(b)) => {
                assert!(
                    (a - b).abs() <= 1e-12 * b.abs().max(1.0),
                    "{src}: {a} vs {b}"
                );
            }
            _ => assert_eq!(got, want, "{src}"),
        }
    }
    // NORM.S.DIST(0, BLANK()) is the density (BLANK flag = FALSE).
    let pdf = f.number("NORM.S.DIST(0, BLANK())");
    assert!((pdf - 1.0 / (2.0 * std::f64::consts::PI).sqrt()).abs() < 1e-15);
}

#[test]
fn holidays_come_from_a_table_expression() {
    let f = fx();
    // Dec 23 2024 (Mon) to Jan 3 2025 (Fri): 10 weekdays, minus the three
    // weekday holidays in T[Day] (the Saturday one does not count twice).
    assert_eq!(
        f.value("NETWORKDAYS(DATE(2024, 12, 23), DATE(2025, 1, 3), 1, VALUES(T[Day]))"),
        Value::Integer(7)
    );
    assert_eq!(
        f.value(
            "NETWORKDAYS(DATE(2024, 12, 23), DATE(2025, 1, 3), 1, FILTER(VALUES(T[Day]), FALSE()))"
        ),
        Value::Integer(10)
    );
    // Row context: one value per row, inside an iterator.
    assert_eq!(
        f.value("SUMX(FILTER(T, NOT ISBLANK(T[Day])), NETWORKDAYS(DATE(2024, 12, 1), T[Day]))"),
        Value::Integer(18 + 19 + 23 + 20)
    );
    // Wide ranges are counted arithmetically.
    assert_eq!(
        f.value("NETWORKDAYS(DATE(1900, 1, 1), DATE(9999, 12, 31), 11)"),
        Value::Integer(2_958_464 - 422_637)
    );
}

#[test]
fn rand_is_deterministic_per_evaluation() {
    let f = fx();
    let first = f.value("RAND()");
    assert_eq!(f.value("RAND()"), first);
    assert_eq!(f.value("RAND() <> RAND()"), Value::Boolean(true));
    // Every row of an iteration gets its own number, all in [0, 1).
    let distinct = f.number(
        "COUNTROWS(DISTINCT(SELECTCOLUMNS(ADDCOLUMNS(GENERATESERIES(1, 200), \"r\", RAND()), \"r\", [r])))",
    );
    assert_eq!(distinct, 200.0);
    assert_eq!(
        f.value(
            "MINX(GENERATESERIES(1, 500), RAND()) >= 0 && MAXX(GENERATESERIES(1, 500), RAND()) < 1"
        ),
        Value::Boolean(true)
    );
    let dice = "GENERATESERIES(1, 3000)";
    assert_eq!(f.number(&format!("MINX({dice}, RANDBETWEEN(1, 6))")), 1.0);
    assert_eq!(f.number(&format!("MAXX({dice}, RANDBETWEEN(1, 6))")), 6.0);
    let mean = f.number(&format!("AVERAGEX({dice}, RANDBETWEEN(1, 6))"));
    assert!((mean - 3.5).abs() < 0.15, "{mean}");
    // A calculated column draws one number per row, reproducibly.
    let snap = f.engine.snapshot();
    let rows: Vec<Value> = (0..5)
        .map(|r| snap.value_at("T", "R", r).unwrap())
        .collect();
    let again = compute_calculated_column(&snap, &f.model, "T", "R").unwrap();
    let again = pbi_engine::value::values_from_array(&again).unwrap();
    assert_eq!(rows, again);
    assert!(rows.windows(2).all(|w| w[0] != w[1]), "{rows:?}");
}

/// Every function added by the library wave (and the date functions whose
/// semantics changed).
const NEW_FUNCTIONS: &[&str] = &[
    "PI",
    "SQRTPI",
    "EXP",
    "LN",
    "LOG",
    "LOG10",
    "SIN",
    "COS",
    "TAN",
    "COT",
    "ASIN",
    "ACOS",
    "ATAN",
    "ACOT",
    "SINH",
    "COSH",
    "TANH",
    "COTH",
    "ASINH",
    "ACOSH",
    "ATANH",
    "ACOTH",
    "DEGREES",
    "RADIANS",
    "FACT",
    "COMBIN",
    "COMBINA",
    "PERMUT",
    "GCD",
    "LCM",
    "QUOTIENT",
    "EVEN",
    "ODD",
    "MROUND",
    "CEILING",
    "ISO.CEILING",
    "FLOOR",
    "RAND",
    "RANDBETWEEN",
    "BITAND",
    "BITOR",
    "BITXOR",
    "BITLSHIFT",
    "BITRSHIFT",
    "NORM.DIST",
    "NORM.INV",
    "NORM.S.DIST",
    "NORM.S.INV",
    "EXPON.DIST",
    "POISSON.DIST",
    "BETA.DIST",
    "BETA.INV",
    "CHISQ.DIST",
    "CHISQ.DIST.RT",
    "CHISQ.INV",
    "CHISQ.INV.RT",
    "T.DIST",
    "T.DIST.2T",
    "T.DIST.RT",
    "T.INV",
    "T.INV.2T",
    "CONFIDENCE.NORM",
    "CONFIDENCE.T",
    "GEOMEAN",
    "GEOMEANX",
    "MAXA",
    "MINA",
    "AVERAGEA",
    "YEARFRAC",
    "NETWORKDAYS",
    "EDATE",
    "EOMONTH",
    "QUARTER",
    "IF.EAGER",
    "ERROR",
    "EVALUATEANDLOG",
    "USERCULTURE",
    "ISAFTER",
    "ISONORAFTER",
];

/// Argument shapes for the sweep: BLANK, zero, signs, fractions, extremes,
/// NaN and infinities, text, booleans, dates, columns and tables.
const ARGS: &[&str] = &[
    "BLANK()",
    "0",
    "1",
    "-1",
    "2",
    "2.5",
    "-7.25",
    "0.5",
    "0.975",
    "1E-300",
    "1E300",
    "-1E300",
    "1E10",
    "170",
    "64",
    "-64",
    "9223372036854775807",
    "0 / 0",
    "1 / 0",
    "-1 / 0",
    "\"abc\"",
    "\"\"",
    "\"12\"",
    "\"0000011\"",
    "TRUE()",
    "FALSE()",
    "DATE(2024, 2, 29)",
    "dt\"1900-01-01\"",
    "CURRENCY(1.5)",
    "T[N]",
    "T[Day]",
    "T",
    "{1, 2, 3}",
    "{(1, 2)}",
    "VALUES(T[Day])",
    "ASC",
    "DESC",
];

/// Small deterministic generator for the sweep.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self, n: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % n as u64) as usize
    }
}

#[test]
fn new_functions_never_panic_on_any_argument_shape() {
    let f = fx();
    let mut rng = Lcg(0x5EED);
    let passthrough = ["IF.EAGER", "EVALUATEANDLOG", "ERROR", "EDATE", "EOMONTH"];
    let mut ok = 0usize;
    for name in NEW_FUNCTIONS {
        for _ in 0..400 {
            // Half the calls use 1 to 4 arguments, the usual arities.
            let count = if rng.next(2) == 0 {
                rng.next(7)
            } else {
                1 + rng.next(4)
            };
            // Half the calls draw numbers only (the first 20 shapes).
            let pool = if rng.next(2) == 0 { &ARGS[..20] } else { ARGS };
            let args: Vec<&str> = (0..count).map(|_| pool[rng.next(pool.len())]).collect();
            let src = format!("{name}({})", args.join(", "));
            // Errors are fine; panics and non-finite numbers are not.
            if let Ok(v) = f.eval(&src) {
                ok += 1;
                if let Value::Double(x) = v {
                    assert!(
                        x.is_finite() || passthrough.contains(name),
                        "{src} returned {x}"
                    );
                }
            }
        }
        // In a row context too (column arguments get values).
        let src = format!(
            "SUMX(T, IFERROR({name}(T[N], T[N], T[N]), 0) + IFERROR({name}(T[N]), 0) + IFERROR({name}(T[N], 2), 0))"
        );
        let _ = f.eval(&src);
    }
    assert!(ok > 2_000, "only {ok} successful evaluations");
}
