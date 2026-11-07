//! The TMDL language service against Microsoft's TMDL reader (ground-truth suites
//! `tmdl-catalog` and `tmdl-validity`, ADR-0011).
//!
//! `src/service/catalog.txt` is generated from `reference/groundtruth/recordings/tmdl-catalog.json`
//! (`DASHCRAFT_BLESS=1 cargo test -p pbi-tmdl --test groundtruth` rewrites it). The recording
//! holds the TOM library's verdict on every keyword in five forms in every context of the probe
//! model `reference/groundtruth/tmdl-probe`; the conformance tests rebuild each probe and require
//! the diagnostics to report an error exactly when TOM rejected it, with the differences listed
//! in `reference/groundtruth/deviations.json`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value as Json;

fn reference_root() -> PathBuf {
    std::env::var_os("DASHCRAFT_REFERENCE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../reference"))
}

fn report_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp/groundtruth")
}

fn catalog_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/service/catalog.txt")
}

fn read_json(path: &Path) -> Json {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// The recording of `suite`, or `None` (after printing SKIP) when it is missing.
fn recording(suite: &str) -> Option<Json> {
    let path = reference_root().join(format!("groundtruth/recordings/{suite}.json"));
    if !path.exists() {
        println!("SKIP: {} missing", path.display());
        return None;
    }
    Some(read_json(&path))
}

// ---------------------------------------------------------------------------------------------
// catalog.txt

/// A recorded verdict, classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cell {
    No,
    Yes,
    From(u32),
    Above(u32),
    /// Accepted by the reader (the error is about the probe's value or a reference, or the
    /// object conflicts with the probe model), at a level that was not probed.
    Unprobed,
}

/// Whether TOM's verdict means the keyword is valid in this form.
fn classify(verdict: &str, fixture_level: u32) -> Cell {
    if verdict == "ok" {
        return Cell::Yes;
    }
    if let Some(n) = verdict.strip_prefix("ok>=") {
        return Cell::From(n.parse().unwrap());
    }
    if verdict == "compat" {
        return Cell::Above(fixture_level);
    }
    // Token prefixes, then message fragments.
    const VALUE_OR_CONFLICT: [&str; 3] = ["type:", "DuplicatedProperty:", "TomInternalException:"];
    const VALUE_MESSAGES: [&str; 5] = [
        "The expected target-type",
        "Invalid custom JSON property",
        "does not comply with the",
        "is unexpected for type",
        "Cannot read or find the Type property",
    ];
    if verdict == "unresolved"
        || VALUE_OR_CONFLICT.iter().any(|p| verdict.starts_with(p))
        || VALUE_MESSAGES.iter().any(|p| verdict.contains(p))
    {
        Cell::Unprobed
    } else {
        Cell::No
    }
}

fn value_type(verdict: &str, bare: &str) -> String {
    if bare.starts_with("InvalidName:") {
        return "ref".to_owned();
    }
    match verdict.strip_prefix("type:") {
        Some("Boolean") => "bool".to_owned(),
        Some("Int32") => "int".to_owned(),
        Some("Int64") => "long".to_owned(),
        Some("Double") => "number".to_owned(),
        Some(e) => format!("enum:{e}"),
        None if verdict == "unresolved" || verdict.contains("The expected target-type") => {
            "ref".to_owned()
        }
        None if verdict.contains("Invalid custom JSON property") => "json".to_owned(),
        None if verdict.contains("does not comply with the") => "content".to_owned(),
        None => "text".to_owned(),
    }
}

fn render_cell(c: Cell) -> String {
    match c {
        Cell::No => "-".to_owned(),
        Cell::Yes | Cell::Unprobed => "y".to_owned(),
        Cell::From(n) => n.to_string(),
        Cell::Above(n) => format!(">{n}"),
    }
}

fn fixture_level() -> u32 {
    let path = reference_root().join("groundtruth/tmdl-probe/database.tmdl");
    let text = std::fs::read_to_string(path).unwrap();
    text.lines()
        .find_map(|l| l.trim().strip_prefix("compatibilityLevel:"))
        .and_then(|v| v.trim().parse().ok())
        .expect("the probe model's compatibility level")
}

/// The catalog line of a recorded row (`[keyword, named, bare, colon, eq, namedEq, sample?,
/// sampleVerdict?]`), or `None` when no form is accepted.
fn catalog_row(ctx: &str, row: &[Json], base_level: Option<u32>, level: u32) -> Option<String> {
    let s = |i: usize| row.get(i).and_then(Json::as_str).unwrap_or_default();
    let kw = s(0);
    let verdicts: Vec<&str> = (1..=5).map(s).collect();
    let mut cells: Vec<Cell> = verdicts.iter().map(|v| classify(v, level)).collect();
    // A bare property has an empty value: a type error means the form is rejected.
    if verdicts[1].starts_with("type:") {
        cells[1] = Cell::No;
    }
    // `kw: <value of the right type>` rejected: the property is not allowed here at all.
    if row.len() > 7 && classify(s(7), level) == Cell::No {
        for i in [1, 2, 3] {
            cells[i] = Cell::No;
        }
    }
    if cells.iter().all(|c| *c == Cell::No) {
        return None;
    }
    // Forms rejected for their value were not level-probed: they need what the probed forms need.
    let probed_level = if cells.contains(&Cell::Yes) {
        None
    } else {
        cells
            .iter()
            .filter_map(|c| match c {
                Cell::From(n) => Some(*n),
                _ => None,
            })
            .min()
    };
    for c in &mut cells {
        if *c == Cell::Unprobed {
            *c = probed_level.map_or(Cell::Yes, Cell::From);
        }
        // The context's own object needs this level already.
        if let (Cell::From(n), Some(base)) = (*c, base_level)
            && n <= base
        {
            *c = Cell::Yes;
        }
    }
    let [named_v, bare_v, colon_v, eq_v, neq_v] = verdicts[..] else {
        unreachable!()
    };
    let accepted = |i: usize| cells[i] != Cell::No;
    let property = accepted(2);
    let object = accepted(0)
        || accepted(4)
        || named_v == "notnamed"
        || neq_v == "notnamed"
        || bare_v == "named"
        || eq_v == "named"
        || eq_v == "nodefault"
        || neq_v == "nodefault"
        || !property;
    let kind = match (object, property) {
        (true, true) => "both",
        (false, true) => "property",
        _ => "object",
    };
    let value = if property {
        value_type(colon_v, bare_v)
    } else {
        "-".to_owned()
    };
    let default = if kind == "property" {
        "-".to_owned()
    } else {
        let eq = if accepted(3) {
            Some(eq_v)
        } else if accepted(4) {
            Some(neq_v)
        } else {
            None
        };
        match eq {
            // `unresolved`: the probe's name `abc` (a table permission's table) is missing.
            Some(v) if v == "ok" || v.starts_with("ok>=") || v == "unresolved" => "expr".to_owned(),
            Some(v) => value_type(v, ""),
            None => "-".to_owned(),
        }
    };
    let mut line = format!("kw\t{ctx}\t{kw}\t{kind}\t{value}\t{default}");
    for c in cells {
        line.push('\t');
        line.push_str(&render_cell(c));
    }
    Some(line)
}

fn render_catalog(rec: &Json) -> String {
    let level = fixture_level();
    let mut out = String::new();
    out.push_str(
        "# TMDL keywords accepted by Microsoft's TMDL reader, per context. Generated from\n",
    );
    out.push_str("# reference/groundtruth/recordings/tmdl-catalog.json; do not edit: run\n");
    out.push_str("# DASHCRAFT_BLESS=1 cargo test -p pbi-tmdl --test groundtruth\n");
    out.push_str("# ctx <context> <level the context's object needs or ->\n");
    out.push_str(
        "# kw <context> <keyword> <object|property|both> <value type> <default value type>\n",
    );
    out.push_str("#    <kw name> <kw> <kw: value> <kw = value> <kw name = value>\n");
    out.push_str("#    (- rejected, y accepted, N from compatibility level N, >N above level N)\n");
    out.push_str(
        "# enum <TOM enumeration> <values accepted by name> <integers of -1 .. 20 accepted>\n",
    );
    let _ = writeln!(
        out,
        "oracle\t{}",
        rec["oracle"]["version"].as_str().unwrap_or("?")
    );
    let cases = rec["cases"].as_array().unwrap();
    let mut enums_used = BTreeSet::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let Some(ctx) = id.strip_prefix("keywords/") else {
            continue;
        };
        assert_eq!(
            case["baseline"], "ok",
            "{id}: the probe model with the context must load"
        );
        let base_level = case["baselineLevel"].as_u64().map(|n| n as u32);
        let _ = writeln!(
            out,
            "ctx\t{ctx}\t{}",
            base_level.map_or("-".to_owned(), |n| n.to_string())
        );
        for row in case["keywords"].as_array().unwrap() {
            if let Some(line) = catalog_row(ctx, row.as_array().unwrap(), base_level, level) {
                for t in line.split('\t').filter_map(|c| c.strip_prefix("enum:")) {
                    enums_used.insert(t.to_owned());
                }
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    let mut enums_recorded = BTreeSet::new();
    for case in cases {
        let id = case["id"].as_str().unwrap();
        let Some(ty) = id.strip_prefix("values/") else {
            continue;
        };
        enums_recorded.insert(ty.to_owned());
        let values: Vec<&str> = case["values"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| {
                let verdict = v[1].as_str().unwrap_or_default();
                verdict == "ok" || verdict.contains("does not comply with the")
            })
            .filter_map(|v| v[0].as_str())
            .collect();
        let _ = writeln!(out, "enum\t{ty}\t{}", values.join("\t"));
    }
    let missing: Vec<_> = enums_used.difference(&enums_recorded).collect();
    assert!(
        missing.is_empty(),
        "enumerations without a values case (add them to TMDL_ENUMS in scripts/groundtruth/gen_corpus.py): {missing:?}"
    );
    out
}

#[test]
fn catalog_is_generated_from_the_tom_recording() {
    let Some(rec) = recording("tmdl-catalog") else {
        return;
    };
    let expected = render_catalog(&rec);
    if std::env::var_os("DASHCRAFT_BLESS").is_some() {
        std::fs::write(catalog_path(), &expected).unwrap();
        println!("wrote {}", catalog_path().display());
        return;
    }
    let actual = std::fs::read_to_string(catalog_path()).unwrap();
    assert!(
        actual.replace("\r\n", "\n") == expected,
        "src/service/catalog.txt is stale: run DASHCRAFT_BLESS=1 cargo test -p pbi-tmdl --test groundtruth"
    );
    let catalog = pbi_tmdl::service::catalog();
    let lines = expected.lines().filter(|l| l.starts_with("kw\t")).count();
    let parsed: usize = catalog.contexts.iter().map(|c| c.keywords.len()).sum();
    assert_eq!(parsed, lines, "every kw line of catalog.txt parses");
    assert_eq!(
        Some(catalog.oracle_version),
        rec["oracle"]["version"].as_str()
    );
}

// ---------------------------------------------------------------------------------------------
// Diagnostics against the reader

/// The `.tmdl` files under `dir` as (path relative to `dir` with `/`, text with LF line ends).
fn read_model(dir: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().collect();
        entries.sort_by_key(|e| e.path());
        for e in entries {
            let path = e.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else if path.extension().is_some_and(|x| x == "tmdl") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = std::fs::read_to_string(&path)
                    .unwrap()
                    .replace("\r\n", "\n");
                out.push((rel, text));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out
}

/// A model folder with its files parsed once; one file at a time is swapped for a probe.
struct Workbench {
    files: Vec<(String, String)>,
    sources: Vec<pbi_tmdl::service::Source>,
}

impl Workbench {
    fn new(dir: &Path) -> Self {
        let files = read_model(dir);
        let sources = files
            .iter()
            .map(|(p, t)| pbi_tmdl::service::Source::new(p.clone(), t.clone()))
            .collect();
        Workbench { files, sources }
    }

    fn text(&self, rel: &str) -> Option<&str> {
        self.files
            .iter()
            .find(|(p, _)| p == rel)
            .map(|(_, t)| t.as_str())
    }

    /// The error diagnostics with `edits` (path → new text; new paths are added) applied.
    fn errors(&mut self, edits: &[(&str, String)]) -> Vec<pbi_tmdl::service::TmdlDiagnostic> {
        let mut added = 0;
        let mut swapped = Vec::new();
        for (rel, text) in edits {
            let source = pbi_tmdl::service::Source::new(*rel, text.clone());
            match self.files.iter().position(|(p, _)| p == rel) {
                Some(i) => swapped.push((i, std::mem::replace(&mut self.sources[i], source))),
                None => {
                    self.sources.push(source);
                    added += 1;
                }
            }
        }
        let out = pbi_tmdl::service::diagnostics(&self.sources)
            .into_iter()
            .filter(|d| d.severity == pbi_tmdl::service::Severity::Error)
            .collect();
        self.sources.truncate(self.sources.len() - added);
        for (i, source) in swapped {
            self.sources[i] = source;
        }
        out
    }
}

/// `field: value` lines of a corpus query (`\t`, `\n` and `\\` unescaped).
fn read_spec(query: &str) -> BTreeMap<String, String> {
    let unescape = |s: &str| {
        let mut out = String::new();
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('t') => out.push('\t'),
                    Some('n') => out.push('\n'),
                    Some(other) => out.push(other),
                    None => out.push('\\'),
                }
            } else {
                out.push(c);
            }
        }
        out
    };
    let mut spec = BTreeMap::new();
    for line in query.split('\n') {
        if let Some((k, v)) = line.split_once(": ") {
            spec.insert(k.to_owned(), unescape(v));
        } else if let Some(k) = line.strip_suffix(':') {
            spec.insert(k.to_owned(), String::new());
        }
    }
    spec
}

/// The probed file's text with `lines` inserted after the anchor (as Record-Tmdl.ps1 does).
fn probe_text(original: &str, spec: &BTreeMap<String, String>, lines: &[&str]) -> String {
    let anchor = format!("{}\n", spec["after"]);
    let at = original.find(&anchor).expect("anchor") + anchor.len();
    assert!(
        !original[at..].contains(&anchor),
        "anchor {anchor:?} occurs more than once"
    );
    let mut depth: usize = spec["depth"].parse().unwrap();
    let mut insert = String::new();
    if let Some(prefix) = spec.get("prefix") {
        for p in prefix.split('\n') {
            let _ = writeln!(insert, "{}{p}", "\t".repeat(depth));
        }
        depth += 1;
    }
    for l in lines {
        let _ = writeln!(insert, "{}{l}", "\t".repeat(depth));
    }
    format!("{}{insert}{}", &original[..at], &original[at..])
}

fn set_level(database: &str, level: u32) -> String {
    let at = database
        .find("compatibilityLevel: ")
        .expect("compatibilityLevel")
        + 20;
    let end = database[at..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(database.len(), |i| at + i);
    format!("{}{level}{}", &database[..at], &database[end..])
}

fn codes_of(errors: &[pbi_tmdl::service::TmdlDiagnostic]) -> String {
    if errors.is_empty() {
        return "no error".to_owned();
    }
    let codes: Vec<String> = errors
        .iter()
        .take(3)
        .map(|d| format!("{} {}:{} {}", d.code, d.path, d.line, d.message))
        .collect();
    codes.join("; ")
}

/// Whether TOM loaded the probe at the probe model's level.
fn tom_accepts(verdict: &str, level: u32) -> bool {
    verdict == "ok"
        || verdict
            .strip_prefix("ok>=")
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| n <= level)
}

#[derive(Default)]
struct Outcome {
    cases: usize,
    mismatches: Vec<(String, String)>,
}

impl Outcome {
    fn check(
        &mut self,
        id: String,
        tom_ok: bool,
        verdict: &str,
        errors: &[pbi_tmdl::service::TmdlDiagnostic],
    ) {
        self.cases += 1;
        if tom_ok != errors.is_empty() {
            self.mismatches.push((
                id,
                format!("TOM: {verdict}; DashCraft: {}", codes_of(errors)),
            ));
        }
    }
}

#[derive(Debug, Clone)]
struct Deviation {
    case: String,
    kind: String,
}

impl Deviation {
    fn covers(&self, id: &str) -> bool {
        match self.case.strip_suffix('*') {
            Some(prefix) => id.starts_with(prefix),
            None => self.case == id,
        }
    }
}

const DEVIATION_KINDS: [&str; 4] = ["bug", "unsupported", "byDesign", "lenient"];

fn ledger(suite: &str) -> Vec<Deviation> {
    let path = reference_root().join("groundtruth/deviations.json");
    if !path.exists() {
        return Vec::new();
    }
    read_json(&path)["deviations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["suite"] == suite)
        .map(|d| {
            let dev = Deviation {
                case: d["case"].as_str().unwrap_or_default().to_owned(),
                kind: d["kind"].as_str().unwrap_or_default().to_owned(),
            };
            assert!(
                DEVIATION_KINDS.contains(&dev.kind.as_str()),
                "deviations.json: {suite}/{}: kind must be one of {DEVIATION_KINDS:?}",
                dev.case
            );
            assert!(
                d["reason"].as_str().is_some_and(|r| !r.trim().is_empty()),
                "deviations.json: {suite}/{}: a reason is required",
                dev.case
            );
            dev
        })
        .collect()
}

/// Writes the report and fails on mismatches the ledger does not list (and on stale entries).
fn finish(suite: &str, rec: &Json, outcome: &Outcome) {
    let deviations = ledger(suite);
    let mismatches = &outcome.mismatches;
    let new: Vec<_> = mismatches
        .iter()
        .filter(|(id, _)| !deviations.iter().any(|d| d.covers(id)))
        .collect();
    let stale: Vec<_> = deviations
        .iter()
        .filter(|d| !mismatches.iter().any(|(id, _)| d.covers(id)))
        .collect();
    let matched = outcome.cases - mismatches.len();
    let mut md = String::new();
    let _ = writeln!(md, "# Ground truth: {suite}\n");
    let _ = writeln!(
        md,
        "{} {}, recorded {}.\n",
        rec["oracle"]["library"].as_str().unwrap_or("?"),
        rec["oracle"]["version"].as_str().unwrap_or("?"),
        rec["recordedOn"].as_str().unwrap_or("?")
    );
    let _ = writeln!(
        md,
        "| checks | match | listed deviations | new mismatches | stale ledger entries |\n|---|---|---|---|---|\n| {} | {matched} | {} | {} | {} |\n",
        outcome.cases,
        mismatches.len() - new.len(),
        new.len(),
        stale.len()
    );
    if !mismatches.is_empty() {
        let _ = writeln!(md, "| case | ledger | detail |\n|---|---|---|");
        for (id, detail) in mismatches {
            let listed = deviations
                .iter()
                .find(|d| d.covers(id))
                .map_or("**new**".to_owned(), |d| d.kind.clone());
            let _ = writeln!(md, "| `{id}` | {listed} | {} |", detail.replace('|', "\\|"));
        }
    }
    let dir = report_dir();
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join(format!("{suite}.md")), &md);
    println!(
        "{suite}: {} checks, {matched} match, {} listed, {} new, {} stale; report {}",
        outcome.cases,
        mismatches.len() - new.len(),
        new.len(),
        stale.len(),
        dir.join(format!("{suite}.md")).display()
    );
    let mut failures = Vec::new();
    for (id, detail) in &new {
        failures.push(format!("  NEW {id}: {detail}"));
    }
    for d in &stale {
        failures.push(format!(
            "  STALE ledger entry {} ({}): it matches the reader now; remove it",
            d.case, d.kind
        ));
    }
    assert!(
        failures.is_empty(),
        "ground truth {suite}: {} problem(s) (fix them, or list them in reference/groundtruth/deviations.json with a reason):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The recording of `suite` when its corpus queries match the committed corpus.
fn checked_recording(suite: &str) -> Option<Json> {
    let rec = recording(suite)?;
    let corpus_path = reference_root().join(format!("groundtruth/corpus/{suite}.json"));
    if !corpus_path.exists() {
        println!("SKIP: {} missing", corpus_path.display());
        return None;
    }
    let corpus = read_json(&corpus_path);
    let recorded: BTreeMap<&str, &Json> = rec["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["id"].as_str().unwrap(), &c["query"]))
        .collect();
    for case in corpus["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        assert_eq!(
            recorded.get(id).copied(),
            Some(&case["query"]),
            "{suite}/{id}: not recorded or the query changed; record again (scripts/groundtruth/Record-Tmdl.ps1)"
        );
    }
    Some(rec)
}

const FORMS: [(&str, &str); 5] = [
    ("named", "{} abc"),
    ("bare", "{}"),
    ("colon", "{}: abc"),
    ("eq", "{} = 1"),
    ("namedEq", "{} abc = 1"),
];

/// Every recorded keyword probe (five forms in every context, the lowest accepting level) and
/// enum value: an error exactly where the reader rejected the probe.
#[test]
fn diagnostics_match_the_reader_on_every_keyword() {
    let Some(rec) = checked_recording("tmdl-catalog") else {
        return;
    };
    let level = fixture_level();
    let mut bench = Workbench::new(&reference_root().join("groundtruth/tmdl-probe"));
    let database = bench.text("database.tmdl").unwrap().to_owned();
    let mut outcome = Outcome::default();
    for case in rec["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let spec = read_spec(case["query"].as_str().unwrap());
        let rel = spec["file"].clone();
        let original = bench.text(&rel).unwrap().to_owned();
        let mut run = |lines: &[&str], at_level: Option<u32>| {
            let mut text = probe_text(&original, &spec, lines);
            let mut edits = Vec::new();
            if let Some(l) = at_level {
                if rel == "database.tmdl" {
                    text = set_level(&text, l);
                } else {
                    edits.push(("database.tmdl", set_level(&database, l)));
                }
            }
            edits.push((rel.as_str(), text));
            bench.errors(&edits)
        };
        if id.starts_with("keywords/") {
            let baseline = run(&[], None);
            outcome.check(format!("{id}/baseline"), true, "ok", &baseline);
            for row in case["keywords"].as_array().unwrap() {
                let row = row.as_array().unwrap();
                let kw = row[0].as_str().unwrap();
                for (i, (form, pattern)) in FORMS.iter().enumerate() {
                    let verdict = row[i + 1].as_str().unwrap();
                    let line = pattern.replace("{}", kw);
                    let errors = run(&[&line], None);
                    outcome.check(
                        format!("{id}/{kw}/{form}"),
                        tom_accepts(verdict, level),
                        verdict,
                        &errors,
                    );
                    if let Some(n) = verdict
                        .strip_prefix("ok>=")
                        .and_then(|n| n.parse::<u32>().ok())
                    {
                        let errors = run(&[&line], Some(n - 1));
                        outcome.check(
                            format!("{id}/{kw}/{form}@{}", n - 1),
                            false,
                            "compat",
                            &errors,
                        );
                    }
                }
                if let (Some(sample), Some(verdict)) = (
                    row.get(6).and_then(Json::as_str),
                    row.get(7).and_then(Json::as_str),
                ) {
                    let line = format!("{kw}: {sample}");
                    let errors = run(&[&line], None);
                    outcome.check(
                        format!("{id}/{kw}/sample"),
                        tom_accepts(verdict, level),
                        verdict,
                        &errors,
                    );
                }
            }
        } else if id.starts_with("values/") {
            let kw = spec["keyword"].clone();
            for value in case["values"].as_array().unwrap() {
                let v = value[0].as_str().unwrap();
                let verdict = value[1].as_str().unwrap();
                let line = format!("{kw}: {v}");
                let errors = run(&[&line], None);
                outcome.check(
                    format!("{id}/{v}"),
                    tom_accepts(verdict, level),
                    verdict,
                    &errors,
                );
            }
        }
    }
    finish("tmdl-catalog", &rec, &outcome);
}

/// Edited copies of the Acme Outfitters model: an error exactly where the reader rejected the
/// edit, on the line the reader reported.
#[test]
fn diagnostics_match_the_reader_on_edited_models() {
    let Some(rec) = checked_recording("tmdl-validity") else {
        return;
    };
    let mut bench = Workbench::new(
        &reference_root().join("examples/acme-outfitters/AcmeOutfitters.SemanticModel/definition"),
    );
    let mut outcome = Outcome::default();
    for case in rec["cases"].as_array().unwrap() {
        let id = case["id"].as_str().unwrap();
        let spec = read_spec(case["query"].as_str().unwrap());
        let rel = spec["file"].as_str();
        let find = spec["find"].as_str();
        let text = if find.is_empty() {
            spec["replace"].clone()
        } else {
            let original = bench.text(rel).unwrap();
            assert_eq!(
                original.matches(find).count(),
                1,
                "{id}: find text not found once"
            );
            original.replacen(find, &spec["replace"], 1)
        };
        let errors = bench.errors(&[(rel, text)]);
        let verdict = case["verdict"].as_str().unwrap();
        let tom_ok = verdict == "ok";
        outcome.check(id.to_owned(), tom_ok, verdict, &errors);
        if let (Some(doc), Some(line)) = (case["document"].as_str(), case["line"].as_u64())
            && !tom_ok
            && !errors.is_empty()
            && !id.starts_with("compat/")
        {
            outcome.cases += 1;
            if !errors
                .iter()
                .any(|d| d.path == doc && u64::from(d.line) == line)
            {
                outcome.mismatches.push((
                    format!("{id}@line"),
                    format!("TOM: {doc}:{line}; DashCraft: {}", codes_of(&errors)),
                ));
            }
        }
    }
    finish("tmdl-validity", &rec, &outcome);
}

/// Models the reader loads (the examples and fixtures, Desktop's own output) have no errors.
#[test]
fn diagnostics_report_no_error_on_models_the_reader_loads() {
    let root = reference_root();
    let folders = [
        "examples/acme-outfitters/AcmeOutfitters.SemanticModel/definition",
        "groundtruth/tmdl-probe",
        "groundtruth/fixtures/dax-lab/DaxLab.SemanticModel/definition",
        "groundtruth/gt4/s24/before",
    ];
    for folder in folders {
        let dir = root.join(folder);
        if !dir.exists() {
            println!("SKIP: {} missing", dir.display());
            continue;
        }
        let mut bench = Workbench::new(&dir);
        let all = pbi_tmdl::service::diagnostics(&bench.sources);
        assert!(all.is_empty(), "{folder}: {all:#?}");
        assert!(bench.errors(&[]).is_empty());
    }
}
