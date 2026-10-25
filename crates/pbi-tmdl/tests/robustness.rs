//! Malformed input never panics: it yields a syntax error or diagnostics.
//! A deterministic property test mutates the fixture files (line shuffles,
//! swaps, deletions, duplications, truncations, junk insertion, indentation
//! changes) and checks that parsing never panics, the CST stays lossless,
//! the writer's output always parses, and model → TMDL → model is stable.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

use pbi_model::{Model, PropValue, Severity};
use pbi_tmdl::{
    SyntaxError, TmdlError, clear_origins, codes, load_model, parse_file, write_model,
    write_model_preserving,
};

mod common;

fn syntax_kind(text: &str) -> SyntaxError {
    match parse_file("bad.tmdl", text) {
        Err(TmdlError::Syntax { kind, .. }) => kind,
        other => panic!("expected a syntax error for {text:?}, got {other:?}"),
    }
}

#[test]
fn malformed_inputs_are_errors_not_panics() {
    assert!(matches!(
        syntax_kind("table A\n\t\tcolumn B\n"),
        SyntaxError::UnexpectedIndentation { .. }
    ));
    assert!(matches!(
        syntax_kind("\ttable A\n"),
        SyntaxError::UnexpectedIndentation { .. }
    ));
    assert_eq!(
        syntax_kind("table 'Web Sessions\n"),
        SyntaxError::UnterminatedName
    );
    assert_eq!(
        syntax_kind("table Web Sessions\n"),
        SyntaxError::UnexpectedText
    );
    assert_eq!(syntax_kind("table A\n\t)\n"), SyntaxError::ExpectedKeyword);
    assert_eq!(
        syntax_kind("table A\n\tmeasure M = ```\n\t\t\tx\n"),
        SyntaxError::UnterminatedFence
    );
    assert!(matches!(
        syntax_kind("table A\n    column B\n      x\n"),
        SyntaxError::PartialIndentation { .. }
    ));
    let err = parse_file("tables/A.tmdl", "table A\n\tcolumn 'B\n").unwrap_err();
    assert_eq!(
        err.to_string(),
        "tables/A.tmdl:2:9: unterminated quoted name (missing closing `'`)"
    );
}

#[test]
fn empty_and_trivia_only_files() {
    for text in [
        "",
        "\n",
        "\r\n\r\n",
        "\u{feff}",
        "   \n\t\n",
        "// note\n/// orphan\n",
    ] {
        let f = parse_file("e.tmdl", text).unwrap();
        assert!(f.nodes.is_empty());
        assert_eq!(f.to_string(), text);
    }
    let loaded = load_model(&[("e.tmdl".into(), "/// orphan\n".into())]).unwrap();
    assert_eq!(loaded.model, Model::default());
    assert_eq!(loaded.diagnostics[0].code, codes::ORPHAN_DESCRIPTION);
    assert!(load_model(&[]).unwrap().files.is_empty());
}

#[test]
fn unknown_keywords_are_preserved_with_info_diagnostics() {
    let text = "futureThing Foo = 42\n\tsetting: on\n\ntable A\n\tsparkle\n\tglitter: 'x'\n\n\tcolumn B\n\t\tdataType: string\n\t\tisHidden: maybe\n\t\tnewProperty = 1 + 2\n";
    let loaded = load_model(&[("x.tmdl".into(), text.into())]).unwrap();
    let m = &loaded.model;
    let PropValue::Object(thing) = &m.extra["futureThing"] else {
        panic!("futureThing");
    };
    assert_eq!(thing.name.as_deref(), Some("Foo"));
    assert_eq!(thing.expression.as_deref(), Some("42"));
    assert_eq!(thing.properties["setting"], PropValue::Text("on".into()));
    let t = m.table("A").unwrap();
    assert_eq!(t.extra["sparkle"], PropValue::Bool(true));
    assert_eq!(t.extra["glitter"], PropValue::Text("'x'".into()));
    let c = t.column("B").unwrap();
    assert!(!c.is_hidden);
    assert_eq!(c.extra["isHidden"], PropValue::Text("maybe".into()));
    assert_eq!(
        c.extra["newProperty"],
        PropValue::Expression("1 + 2".into())
    );
    let infos = loaded
        .diagnostics
        .iter()
        .filter(|d| d.code == codes::UNMODELED && d.severity == Severity::Info)
        .count();
    assert_eq!(infos, 4);
    assert!(
        loaded
            .diagnostics
            .iter()
            .any(|d| d.code == codes::INVALID_VALUE && d.severity == Severity::Warning)
    );
    let written = write_model(&loaded.model, None);
    let again = load_model(&written).unwrap();
    assert_eq!(normalized(again.model), normalized(loaded.model));
}

#[test]
fn mixed_tabs_and_spaces_parse_with_a_warning() {
    let text = "table A\n\tcolumn B\n\t    dataType: string\n    column C\n";
    let f = parse_file("m.tmdl", text).unwrap();
    assert_eq!(f.to_string(), text);
    assert_eq!(f.nodes[0].children.len(), 2);
    assert_eq!(f.diagnostics[0].code, codes::MIXED_INDENTATION);
}

#[test]
fn crlf_expressions_have_no_carriage_returns() {
    let text =
        "table A\r\n\tmeasure M =\r\n\t\t\tVAR x = 1\r\n\t\t\tRETURN x\r\n\t\tformatString: 0\r\n";
    let loaded = load_model(&[("a.tmdl".into(), text.into())]).unwrap();
    assert_eq!(loaded.files[0].to_string(), text);
    let m = loaded.model.measure("A", "M").unwrap();
    assert_eq!(m.expression, "VAR x = 1\nRETURN x");
    assert_eq!(m.format_string.as_deref(), Some("0"));
}

fn normalized(mut model: Model) -> Model {
    clear_origins(&mut model);
    if let Some(db) = &mut model.database {
        db.origin = None;
    }
    model
}

/// xorshift64* — deterministic, dependency-free.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn fixture_files() -> Option<Vec<(String, String)>> {
    let root = std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../reference"));
    let dir = root.join("examples/acme-outfitters/AcmeOutfitters.SemanticModel/definition");
    if !dir.join("model.tmdl").is_file() {
        println!("SKIP: {} missing", dir.display());
        return None;
    }
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(&dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.path().extension().is_some_and(|x| x == "tmdl") {
            let rel = entry
                .path()
                .strip_prefix(&dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.push((rel, std::fs::read_to_string(entry.path()).unwrap()));
        }
    }
    files.sort();
    Some(files)
}

/// Applies 1–3 random mutations; `pool` supplies lines to splice in.
fn mutate(rng: &mut Rng, text: &str, pool: &[&str]) -> String {
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_owned).collect();
    const JUNK: &[&str] = &[
        "\t",
        " ",
        "'",
        "=",
        ":",
        "```",
        "///",
        "//",
        "\n",
        "\r\n",
        "\r",
        "x",
        "\"",
        "[",
        "é",
        "ref ",
        "\t\t\t\t\t\t",
        "= ```\n",
        "\u{feff}",
        "\n\tref table X\n",
        "\n\t\ttype: rowNumber\n",
        "\n\tpartition P = entity\n\t\tsource\n\t\t\tentityName: e\n",
        "\n\t\tisHidden: false\n",
        "\n\tcolumn Z =\n",
        "\n\t\tsource = query\n",
        ": ",
        "\n\t\t\t```\n",
    ];
    let rounds = 1 + rng.below(3);
    for _ in 0..rounds {
        let n = lines.len();
        match rng.below(9) {
            0 => {
                for i in (1..n).rev() {
                    lines.swap(i, rng.below(i + 1));
                }
            }
            1 if n > 1 => {
                let (a, b) = (rng.below(n), rng.below(n));
                lines.swap(a, b);
            }
            2 if n > 0 => {
                lines.remove(rng.below(n));
            }
            3 if n > 0 => {
                let i = rng.below(n);
                let dup = lines[i].clone();
                lines.insert(rng.below(n + 1), dup);
            }
            4 => {
                let joined = lines.concat();
                let mut cut = rng.below(joined.len() + 1);
                while !joined.is_char_boundary(cut) {
                    cut -= 1;
                }
                lines = vec![joined[..cut].to_owned()];
            }
            5 if n > 0 => {
                let i = rng.below(n);
                let line = &mut lines[i];
                let mut at = rng.below(line.len() + 1);
                while !line.is_char_boundary(at) {
                    at -= 1;
                }
                line.insert_str(at, JUNK[rng.below(JUNK.len())]);
            }
            8 if !pool.is_empty() => {
                let line = pool[rng.below(pool.len())];
                lines.insert(rng.below(n + 1), format!("{line}\n"));
            }
            6 if n > 0 => {
                let i = rng.below(n);
                if rng.below(2) == 0 {
                    lines[i].insert(0, '\t');
                } else if lines[i].starts_with('\t') {
                    lines[i].remove(0);
                }
            }
            _ if n > 0 => {
                let i = rng.below(n);
                lines[i] = lines[i].replace('\n', "\r\n");
            }
            _ => {}
        }
    }
    lines.concat()
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Cases and seed can be raised for longer local runs:
/// `PBI_TMDL_PROPTEST_CASES=100000 PBI_TMDL_PROPTEST_SEED=7 cargo test -p pbi-tmdl --test robustness`.
#[test]
fn mutated_fixtures_never_panic_and_round_trip() {
    let sample: Vec<(String, String)> = common::DESKTOP_SAMPLE
        .iter()
        .map(|(p, t)| ((*p).to_owned(), (*t).to_owned()))
        .collect();
    let corpora: Vec<Vec<(String, String)>> = match fixture_files() {
        Some(files) => vec![files, sample],
        None => vec![sample],
    };
    let cases = env_u64("PBI_TMDL_PROPTEST_CASES", 3000);
    let mut rng = Rng(env_u64("PBI_TMDL_PROPTEST_SEED", 0x9E37_79B9_7F4A_7C15).max(1));
    let pool: Vec<&str> = corpora
        .iter()
        .flatten()
        .flat_map(|(_, t)| t.lines())
        .collect();
    let (mut ok, mut errors) = (0, 0);
    for iteration in 0..cases {
        let files = &corpora[rng.below(corpora.len())];
        let mut set = files.clone();
        let target = rng.below(set.len());
        set[target].1 = mutate(&mut rng, &set[target].1, &pool);
        let mutated = &set[target];
        match parse_file(&mutated.0, &mutated.1) {
            Ok(file) => assert_eq!(file.to_string(), mutated.1, "iteration {iteration}"),
            Err(TmdlError::Syntax { .. }) => {
                errors += 1;
                continue;
            }
            Err(e) => panic!("iteration {iteration}: unexpected error {e}"),
        }
        let loaded = load_model(&set).unwrap();
        let written = write_model(&loaded.model, loaded.database.as_ref());
        let reloaded = match load_model(&written) {
            Ok(r) => r,
            Err(e) => panic!(
                "iteration {iteration}: writer output does not parse: {e}\ninput {:?}:\n{}\nwritten:\n{written:#?}",
                mutated.0, mutated.1
            ),
        };
        let preserved =
            write_model_preserving(&loaded.model, loaded.database.as_ref(), &loaded.files);
        let preserved =
            load_model(&preserved).unwrap_or_else(|e| panic!("iteration {iteration}: {e}"));
        let (a, b) = (normalized(loaded.model), normalized(reloaded.model));
        pretty_assertions::assert_eq!(
            &a,
            &normalized(preserved.model),
            "iteration {iteration}: preserving writer changed the model; input {:?}: {:?}",
            mutated.0,
            mutated.1
        );
        pretty_assertions::assert_eq!(
            a,
            b,
            "iteration {iteration}: model changed through the writer; input {:?}: {:?}",
            mutated.0,
            mutated.1
        );
        ok += 1;
    }
    assert!(ok > 100 && errors > 100, "ok {ok}, errors {errors}");
}
