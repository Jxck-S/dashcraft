//! `dashcraft inspect`: the model, reports and diagnostics of an input, as a
//! human-readable tree or as the stable JSON schema of
//! [`inspect_model`](crate::inspect_model).

use std::io::{self, Write};
use std::path::PathBuf;

use crate::common::{
    Context, ReadArgs, Status, first_line, one_line, open_document, print_json, with_stdout,
};
use crate::inspect_model::{
    self, BuildOptions, ColumnOut, ConnectorOut, CustomVisualOut, DiagnosticOut, FilterOut,
    Inspect, MashupOut, ModelOut, PageOut, PaginatedReportOut, RdlItemOut, ReportOut, TableOut,
    VisualOut, VpaxStatsOut,
};

/// Longest DAX/M snippet shown in the tree.
const EXPR_WIDTH: usize = 100;
/// Longest description snippet shown in the tree.
const DESC_WIDTH: usize = 90;

#[derive(Debug, clap::Args)]
pub struct InspectArgs {
    /// File or folder to inspect: .pbip, .pbix, .pbit, *.SemanticModel,
    /// *.Report, model.bim, a TMDL folder, an unzipped PBIX folder, …
    pub path: PathBuf,
    /// Print JSON (stable schema, `schemaVersion` 1) instead of a tree.
    #[arg(long)]
    pub json: bool,
    /// Pretty-print the JSON (implies --json).
    #[arg(long)]
    pub pretty: bool,
    /// Leave out DAX, M and query text (measures, calculated columns and
    /// tables, partitions, parameters, role filters), e.g. for sharing.
    #[arg(long)]
    pub no_expressions: bool,
    #[command(flatten)]
    pub read: ReadArgs,
}

pub fn run(args: &InspectArgs, ctx: &Context) -> anyhow::Result<Status> {
    let opened = open_document(&args.path, &args.read.open_options())?;
    let shown = args.path.to_string_lossy();
    let out = inspect_model::build(
        &shown,
        &opened,
        BuildOptions {
            expressions: !args.no_expressions,
        },
    );
    let json = args.json || args.pretty;
    if json {
        print_json(&out, args.pretty)?;
    } else {
        with_stdout(|w| write_tree(w, &out))?;
    }
    let status = Status::of(&opened.document);
    if json && status == Status::ErrorDiagnostics {
        ctx.note(format!(
            "{shown}: {} error diagnostic(s), {} warning(s)",
            out.counts.errors, out.counts.warnings
        ));
    }
    Ok(status)
}

// ---------------------------------------------------------------------------
// Human-readable tree
// ---------------------------------------------------------------------------

fn write_tree(w: &mut dyn Write, out: &Inspect) -> io::Result<()> {
    let src = &out.source;
    writeln!(w, "{}", src.path)?;
    writeln!(
        w,
        "  Format: {} ({}), confidence {}",
        src.detection.label, src.format, src.detection.confidence
    )?;
    if let Some(v) = &src.package_version {
        writeln!(w, "  Package version: {v}")?;
    }
    if !src.detection.signals.is_empty() {
        writeln!(w, "  Signals:")?;
        for s in &src.detection.signals {
            writeln!(w, "    - {s}")?;
        }
    }

    writeln!(w)?;
    match &out.model {
        Some(m) => write_model(w, m, "Model")?,
        None => {
            let status = &out.model_status;
            match &status.description {
                Some(d) => writeln!(w, "Model: {} ({d})", status.status)?,
                None => writeln!(w, "Model: {}", status.status)?,
            }
        }
    }
    for m in &out.extra_models {
        writeln!(w)?;
        write_model(w, m, "Additional model")?;
    }

    writeln!(w)?;
    writeln!(w, "Reports ({}):", out.reports.len())?;
    for r in &out.reports {
        write_report(w, r)?;
    }

    if let Some(v) = &out.vpax_stats {
        writeln!(w)?;
        write_vpax(w, v)?;
    }
    if let Some(r) = &out.paginated_report {
        writeln!(w)?;
        write_paginated(w, r)?;
    }
    if let Some(m) = &out.mashup {
        writeln!(w)?;
        write_mashup(w, m)?;
    }
    if let Some(v) = &out.custom_visual {
        writeln!(w)?;
        write_custom_visual(w, v)?;
    }
    if let Some(c) = &out.connector {
        writeln!(w)?;
        write_connector(w, c)?;
    }

    writeln!(w)?;
    write_diagnostics(w, &out.diagnostics)?;
    let c = &out.counts;
    writeln!(w)?;
    writeln!(
        w,
        "Totals: tables {}, columns {}, measures {}, relationships {}, reports {}, pages {}, visuals {}",
        c.tables, c.columns, c.measures, c.relationships, c.reports, c.pages, c.visuals
    )
}

fn write_model(w: &mut dyn Write, m: &ModelOut, title: &str) -> io::Result<()> {
    let mut facts = Vec::new();
    if let Some(s) = &m.source {
        if s.folder.is_empty() {
            facts.push(s.kind.to_uppercase());
        } else {
            facts.push(format!("{} in {}", s.kind.to_uppercase(), s.folder));
        }
    }
    if let Some(c) = &m.culture {
        facts.push(format!("culture {c}"));
    }
    if let Some(l) = m.compatibility_level {
        facts.push(format!("compatibility level {l}"));
    }
    if facts.is_empty() {
        writeln!(w, "{title}: {}", m.name)?;
    } else {
        writeln!(w, "{title}: {} ({})", m.name, facts.join(", "))?;
    }
    if let Some(d) = &m.description {
        writeln!(w, "  {}", first_line(d, DESC_WIDTH))?;
    }
    if let Some(c) = &m.cache_abf {
        let state = match c.matches_definition {
            Some(true) => ", matches the definition",
            Some(false) => ", differs from the definition",
            None => "",
        };
        writeln!(
            w,
            "  Local data cache: {} ({}, {} table(s){state})",
            c.path,
            bytes(c.size),
            c.table_count
        )?;
    }

    for t in &m.tables {
        write_table(w, t)?;
    }

    if !m.relationships.is_empty() {
        writeln!(w, "  Relationships ({}):", m.relationships.len())?;
        for r in &m.relationships {
            let from = pbi_model::dax_column_ref(&r.from.table, &r.from.column);
            let to = pbi_model::dax_column_ref(&r.to.table, &r.to.column);
            let mut line = format!(
                "    {from} ({}) → {to} ({}), cross-filter {}",
                r.from_cardinality, r.to_cardinality, r.cross_filtering_behavior
            );
            if !r.is_active {
                line.push_str(", inactive");
            }
            writeln!(w, "{line}")?;
        }
    }

    if !m.expressions.is_empty() {
        writeln!(w, "  Expressions ({}):", m.expressions.len())?;
        for e in &m.expressions {
            let mut kind = if e.is_parameter {
                "parameter".to_owned()
            } else {
                format!("{} query", e.kind.to_uppercase())
            };
            if let Some(t) = &e.parameter_type {
                kind = format!("{kind}, {t}");
            }
            let mut line = format!("    {} ({kind})", e.name);
            if let Some(x) = &e.expression {
                if e.is_parameter {
                    line = format!("{line} = {}", one_line(x, EXPR_WIDTH));
                } else {
                    line = format!("{line}: {}", first_line(x, EXPR_WIDTH));
                }
            }
            writeln!(w, "{line}")?;
            if let Some(d) = &e.description {
                writeln!(w, "      {}", first_line(d, DESC_WIDTH))?;
            }
        }
    }

    if !m.roles.is_empty() {
        writeln!(w, "  Roles ({}):", m.roles.len())?;
        for r in &m.roles {
            match &r.model_permission {
                Some(p) => writeln!(w, "    {} ({p})", r.name)?,
                None => writeln!(w, "    {}", r.name)?,
            }
            for p in &r.table_permissions {
                match &p.filter_expression {
                    Some(x) => writeln!(w, "      {}: {}", p.table, one_line(x, EXPR_WIDTH))?,
                    None => writeln!(w, "      {}", p.table)?,
                }
            }
        }
    }
    Ok(())
}

fn write_table(w: &mut dyn Write, t: &TableOut) -> io::Result<()> {
    let kind = match t.kind.as_str() {
        "calculatedTable" => "calculated table",
        "calculationGroup" => "calculation group",
        "remote" => "remote",
        _ => "table",
    };
    let hidden = if t.is_hidden { ", hidden" } else { "" };
    writeln!(w, "  Table: {} ({kind}{hidden})", t.name)?;
    if let Some(d) = &t.description {
        writeln!(w, "    {}", first_line(d, DESC_WIDTH))?;
    }

    if !t.columns.is_empty() {
        writeln!(w, "    Columns ({}):", t.columns.len())?;
        for c in &t.columns {
            writeln!(w, "      {}", column_line(c))?;
        }
    }

    if !t.measures.is_empty() {
        writeln!(w, "    Measures ({}):", t.measures.len())?;
        for m in &t.measures {
            let mut line = format!("      {}", m.name);
            if let Some(x) = &m.expression {
                line = format!("{line} = {}", one_line(x, EXPR_WIDTH));
            }
            let mut flags = Vec::new();
            if let Some(f) = &m.format_string {
                flags.push(format!("format {f}"));
            }
            if m.is_hidden {
                flags.push("hidden".to_owned());
            }
            if !flags.is_empty() {
                line = format!("{line}  [{}]", flags.join(", "));
            }
            writeln!(w, "{line}")?;
        }
    }

    if !t.partitions.is_empty() {
        writeln!(w, "    Partitions ({}):", t.partitions.len())?;
        for p in &t.partitions {
            let mut line = format!("      {} ({}, {})", p.name, p.kind, p.mode);
            if let Some(e) = &p.entity_name {
                line = format!("{line} entity {e}");
            }
            if let Some(x) = &p.expression {
                line = format!("{line}: {}", one_line(x, EXPR_WIDTH));
            }
            writeln!(w, "{line}")?;
        }
    }

    for h in &t.hierarchies {
        let levels: Vec<&str> = h.levels.iter().map(|l| l.name.as_str()).collect();
        writeln!(w, "    Hierarchy {}: {}", h.name, levels.join(" > "))?;
    }
    Ok(())
}

fn column_line(c: &ColumnOut) -> String {
    let mut facts = vec![c.data_type.clone()];
    match c.kind.as_str() {
        "calculated" => facts.push("calculated".into()),
        "calculatedTableColumn" | "data" => {}
        other => facts.push(other.to_owned()),
    }
    if c.is_hidden {
        facts.push("hidden".into());
    }
    if c.is_key {
        facts.push("key".into());
    }
    let mut line = format!("{} : {}", c.name, facts.join(", "));
    if let Some(x) = &c.expression {
        line = format!("{line} = {}", one_line(x, EXPR_WIDTH));
    }
    if let Some(d) = &c.description {
        line = format!("{line} — {}", first_line(d, DESC_WIDTH));
    }
    line
}

fn write_report(w: &mut dyn Write, r: &ReportOut) -> io::Result<()> {
    let d = &r.dataset_reference;
    let dataset = match (d.kind.as_str(), &d.path) {
        ("byPath", Some(p)) => format!("model byPath {p}"),
        ("byConnection", _) => "remote model (byConnection), not available offline".to_owned(),
        ("embedded", _) => "embedded model".to_owned(),
        _ => "no model reference".to_owned(),
    };
    writeln!(w, "  Report: {} ({dataset})", r.name)?;
    if let Some(t) = &r.base_theme {
        match &r.custom_theme {
            Some(c) => writeln!(w, "    Theme: {t} + {c}")?,
            None => writeln!(w, "    Theme: {t}")?,
        }
    }
    write_filters(w, &r.filters, "    ", "Report filters")?;
    for p in &r.pages {
        write_page(w, p)?;
    }
    Ok(())
}

fn write_page(w: &mut dyn Write, p: &PageOut) -> io::Result<()> {
    let hidden = if p.is_hidden { ", hidden" } else { "" };
    writeln!(
        w,
        "    Page: {} [{}] ({}x{}, {}, {} visuals{hidden})",
        p.display_name,
        p.name,
        num(p.width),
        num(p.height),
        p.display_option,
        p.visuals.len()
    )?;
    write_filters(w, &p.filters, "      ", "Page filters")?;
    for v in &p.visuals {
        write_visual(w, v)?;
    }
    Ok(())
}

fn write_visual(w: &mut dyn Write, v: &VisualOut) -> io::Result<()> {
    let pos = &v.position;
    let mut line = format!(
        "      {} at ({}, {}) {}x{} [{}]",
        v.visual_type,
        num(pos.x),
        num(pos.y),
        num(pos.width),
        num(pos.height),
        v.name
    );
    if let Some(d) = &v.display_name {
        line = format!("{line} \"{d}\"");
    }
    if v.is_hidden {
        line.push_str(", hidden");
    }
    writeln!(w, "{line}")?;
    for f in &v.fields {
        match &f.display_name {
            Some(d) => writeln!(w, "        {}: {} as \"{d}\"", f.role, f.field.reference)?,
            None => writeln!(w, "        {}: {}", f.role, f.field.reference)?,
        }
    }
    write_filters(w, &v.filters, "        ", "Visual filters")
}

fn write_filters(
    w: &mut dyn Write,
    filters: &[FilterOut],
    indent: &str,
    title: &str,
) -> io::Result<()> {
    if filters.is_empty() {
        return Ok(());
    }
    writeln!(w, "{indent}{title} ({}):", filters.len())?;
    for f in filters {
        let field = f
            .field
            .as_ref()
            .map_or_else(|| f.name.clone(), |x| x.reference.clone());
        let mut line = format!("{indent}  {field} ({})", f.kind);
        match &f.condition {
            Some(c) => line = format!("{line}: {c}"),
            None => line.push_str(": (not filtering)"),
        }
        let mut flags = Vec::new();
        if f.is_hidden_in_view_mode {
            flags.push("hidden");
        }
        if f.is_locked_in_view_mode {
            flags.push("locked");
        }
        if !flags.is_empty() {
            line = format!("{line} [{}]", flags.join(", "));
        }
        writeln!(w, "{line}")?;
    }
    Ok(())
}

fn write_diagnostics(w: &mut dyn Write, diagnostics: &[DiagnosticOut]) -> io::Result<()> {
    let count = |s: &str| diagnostics.iter().filter(|d| d.severity == s).count();
    writeln!(
        w,
        "Diagnostics: errors {}, warnings {}, infos {}",
        count("error"),
        count("warning"),
        count("info")
    )?;
    for (severity, title) in [
        ("error", "Errors"),
        ("warning", "Warnings"),
        ("info", "Info"),
    ] {
        let group: Vec<&DiagnosticOut> = diagnostics
            .iter()
            .filter(|d| d.severity == severity)
            .collect();
        if group.is_empty() {
            continue;
        }
        writeln!(w, "  {title}:")?;
        for d in group {
            let mut line = format!("    {} {}", d.code, d.message);
            if let Some(o) = &d.origin {
                let mut at = o.path.clone();
                if let Some(l) = o.line {
                    at = format!("{at}:{l}");
                }
                if let Some(p) = &o.pointer {
                    at = format!("{at}#{p}");
                }
                line = format!("{line} ({at})");
            }
            writeln!(w, "{line}")?;
        }
    }
    Ok(())
}

/// A byte count in binary units, e.g. `1.5 KiB`.
fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KiB", "MiB", "GiB", "TiB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} bytes")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn write_vpax(w: &mut dyn Write, v: &VpaxStatsOut) -> io::Result<()> {
    let mut facts = vec![format!("{} tables", v.tables.len())];
    facts.push(format!("total {}", bytes(v.total_size)));
    if let Some(e) = &v.extractor {
        facts.push(format!("extracted by {e}"));
    }
    if let Some(d) = &v.extraction_date {
        facts.push(format!("on {d}"));
    }
    writeln!(
        w,
        "VertiPaq Analyzer statistics ({}): {}",
        v.source,
        facts.join(", ")
    )?;
    for t in &v.tables {
        let hidden = if t.is_hidden { ", hidden" } else { "" };
        writeln!(
            w,
            "  Table {}: {} rows, {} (columns {}, relationships {}, user hierarchies {}{hidden})",
            t.name,
            t.rows_count,
            bytes(t.table_size),
            bytes(t.columns_size),
            bytes(t.relationships_size),
            bytes(t.user_hierarchies_size)
        )?;
        for c in &t.columns {
            let mut facts = vec![format!("cardinality {}", c.cardinality)];
            facts.push(format!(
                "{} (data {}, dictionary {}, hierarchies {})",
                bytes(c.total_size),
                bytes(c.data_size),
                bytes(c.dictionary_size),
                bytes(c.hierarchies_size)
            ));
            if let Some(e) = &c.encoding {
                facts.push(e.clone());
            }
            if let Some(d) = &c.data_type {
                facts.push(d.clone());
            }
            facts.push(format!("{} segment(s)", c.segments));
            writeln!(w, "    {}: {}", c.name, facts.join(", "))?;
        }
    }
    if !v.relationships.is_empty() {
        writeln!(w, "  Relationships ({}):", v.relationships.len())?;
        for r in &v.relationships {
            let from = pbi_model::dax_column_ref(&r.from.table, &r.from.column);
            let to = pbi_model::dax_column_ref(&r.to.table, &r.to.column);
            writeln!(
                w,
                "    {from} → {to}: {}, {} missing key(s), {} invalid row(s)",
                bytes(r.used_size),
                r.missing_keys,
                r.invalid_rows
            )?;
        }
    }
    Ok(())
}

fn write_rdl_items(w: &mut dyn Write, items: &[RdlItemOut], indent: usize) -> io::Result<()> {
    let pad = " ".repeat(indent);
    for i in items {
        let mut line = format!("{pad}{} {}", i.kind, i.name);
        if let Some(d) = &i.dataset {
            line = format!("{line} [{d}]");
        }
        if let Some(r) = &i.report_name {
            line = format!("{line} → {r}");
        }
        if let Some(v) = &i.value {
            line = format!("{line} = {}", one_line(v, EXPR_WIDTH));
        }
        writeln!(w, "{line}")?;
        write_rdl_items(w, &i.children, indent + 2)?;
    }
    Ok(())
}

fn write_paginated(w: &mut dyn Write, r: &PaginatedReportOut) -> io::Result<()> {
    match &r.schema {
        Some(s) => writeln!(w, "Paginated report (RDL {s}), outline only:")?,
        None => writeln!(w, "Paginated report (RDL), outline only:")?,
    }
    if let Some(d) = &r.description {
        writeln!(w, "  {}", first_line(d, DESC_WIDTH))?;
    }
    writeln!(w, "  Data sources ({}):", r.data_sources.len())?;
    for d in &r.data_sources {
        let mut facts = Vec::new();
        if let Some(p) = &d.data_provider {
            facts.push(p.clone());
        }
        if let Some(s) = &d.shared_reference {
            facts.push(format!("shared {s}"));
        }
        if let Some(c) = &d.connection_string {
            facts.push(format!("connection string {c}"));
        }
        if d.integrated_security == Some(true) {
            facts.push("integrated security".into());
        }
        writeln!(w, "    {} ({})", d.name, facts.join(", "))?;
    }
    writeln!(w, "  Datasets ({}):", r.datasets.len())?;
    for d in &r.datasets {
        let mut line = format!("    {}", d.name);
        if let Some(s) = &d.data_source {
            line = format!("{line} on {s}");
        }
        if let Some(s) = &d.shared_reference {
            line = format!("{line} (shared {s})");
        }
        if let Some(t) = &d.command_type {
            line = format!("{line} [{t}]");
        }
        if let Some(q) = &d.command_text {
            line = format!("{line}: {}", first_line(q, EXPR_WIDTH));
        }
        writeln!(w, "{line}")?;
        if !d.fields.is_empty() {
            let names: Vec<&str> = d.fields.iter().map(|f| f.name.as_str()).collect();
            writeln!(
                w,
                "      Fields: {}",
                one_line(&names.join(", "), EXPR_WIDTH)
            )?;
        }
        for q in &d.query_parameters {
            match &q.value {
                Some(v) => writeln!(
                    w,
                    "      Query parameter {} = {}",
                    q.name,
                    one_line(v, EXPR_WIDTH)
                )?,
                None => writeln!(w, "      Query parameter {}", q.name)?,
            }
        }
    }
    writeln!(w, "  Parameters ({}):", r.parameters.len())?;
    for p in &r.parameters {
        let mut facts = Vec::new();
        if let Some(t) = &p.data_type {
            facts.push(t.clone());
        }
        if let Some(pr) = &p.prompt {
            facts.push(format!("prompt \"{pr}\""));
        }
        if p.multi_value {
            facts.push("multi-value".into());
        }
        if p.hidden {
            facts.push("hidden".into());
        }
        if !p.default_values.is_empty() {
            facts.push(format!("default {}", p.default_values.join(", ")));
        }
        if let Some(d) = &p.default_dataset {
            facts.push(format!("defaults from {d}"));
        }
        if let Some(d) = &p.valid_values_dataset {
            facts.push(format!("values from {d}"));
        }
        writeln!(w, "    {} ({})", p.name, facts.join(", "))?;
    }
    for (i, s) in r.sections.iter().enumerate() {
        let size = match (&s.page_width, &s.page_height) {
            (Some(width), Some(height)) => format!(" ({width} x {height})"),
            _ => String::new(),
        };
        writeln!(w, "  Section {}{size}:", i + 1)?;
        for (title, items) in [
            ("Header", &s.header),
            ("Body", &s.body),
            ("Footer", &s.footer),
        ] {
            if items.is_empty() {
                continue;
            }
            writeln!(w, "    {title}:")?;
            write_rdl_items(w, items, 6)?;
        }
    }
    if r.truncated {
        writeln!(w, "  (report items truncated)")?;
    }
    Ok(())
}

fn write_mashup(w: &mut dyn Write, m: &MashupOut) -> io::Result<()> {
    let kind = match m.kind.as_str() {
        "script" => "M script",
        "template" => "Power Query template",
        "dataflowGen2" => "Dataflow Gen2",
        "dataflowCdm" => "Dataflow (CDM model.json)",
        other => other,
    };
    let mut title = kind.to_owned();
    if let Some(n) = &m.name {
        title = format!("{title} \"{n}\"");
    }
    if let Some(s) = &m.section {
        title = format!("{title}, section {s}");
    }
    writeln!(
        w,
        "Power Query: {title} ({} queries, not evaluated)",
        m.queries.len()
    )?;
    for q in &m.queries {
        let mut flags = Vec::new();
        if let Some(g) = &q.group {
            flags.push(format!("group {g}"));
        }
        if q.entity {
            flags.push("entity".into());
        }
        if q.load_enabled == Some(false) {
            flags.push("load disabled".into());
        }
        if q.hidden == Some(true) {
            flags.push("hidden".into());
        }
        if q.syntax_error {
            flags.push("syntax error".into());
        }
        if flags.is_empty() {
            writeln!(w, "  {}", q.name)?;
        } else {
            writeln!(w, "  {} ({})", q.name, flags.join(", "))?;
        }
    }
    for e in &m.entities {
        writeln!(
            w,
            "  Entity {}: {} attribute(s), {} partition(s)",
            e.name,
            e.attributes,
            e.partitions.len()
        )?;
        for p in &e.partitions {
            match &p.location {
                Some(l) => writeln!(w, "    {} at {}", p.name, one_line(l, EXPR_WIDTH))?,
                None => writeln!(w, "    {}", p.name)?,
            }
        }
    }
    for c in &m.connections {
        writeln!(
            w,
            "  Connection {}: {}",
            c.kind.as_deref().unwrap_or("?"),
            c.path.as_deref().unwrap_or("")
        )?;
    }
    Ok(())
}

fn write_custom_visual(w: &mut dyn Write, v: &CustomVisualOut) -> io::Result<()> {
    let name = v
        .display_name
        .as_deref()
        .or(v.name.as_deref())
        .unwrap_or("(unnamed)");
    let mut facts = Vec::new();
    if let Some(x) = &v.version {
        facts.push(format!("version {x}"));
    }
    if let Some(x) = &v.api_version {
        facts.push(format!("API {x}"));
    }
    if let Some(x) = &v.guid {
        facts.push(format!("guid {x}"));
    }
    if let Some(x) = &v.author {
        facts.push(format!("by {x}"));
    }
    writeln!(
        w,
        "Custom visual: {name} ({}); metadata only, never executed",
        facts.join(", ")
    )?;
    if let Some(d) = &v.description {
        writeln!(w, "  {}", first_line(d, DESC_WIDTH))?;
    }
    let roles: Vec<String> = v
        .data_roles
        .iter()
        .map(|r| match &r.kind {
            Some(k) => format!("{} ({k})", r.name),
            None => r.name.clone(),
        })
        .collect();
    writeln!(w, "  Data roles: {}", roles.join(", "))?;
    if !v.privileges.is_empty() {
        writeln!(w, "  Privileges requested: {}", v.privileges.join(", "))?;
    }
    writeln!(w, "  Script: {} (not run)", bytes(v.script_bytes))
}

fn write_connector(w: &mut dyn Write, c: &ConnectorOut) -> io::Result<()> {
    writeln!(
        w,
        "Custom connector{}: {} file(s); listed only, never executed",
        if c.signed { " (signed)" } else { "" },
        c.files.len()
    )?;
    for s in &c.sources {
        match (&s.section, &s.error) {
            (_, Some(e)) => writeln!(w, "  {}: {}", s.path, first_line(e, DESC_WIDTH))?,
            (Some(sec), None) => writeln!(w, "  {}: section {sec}", s.path)?,
            (None, None) => writeln!(w, "  {}", s.path)?,
        }
        for m in &s.members {
            let mut facts = Vec::new();
            if m.shared {
                facts.push("shared".to_owned());
            }
            if let Some(k) = &m.data_source_kind {
                facts.push(format!("data source {k}"));
            }
            writeln!(w, "    {} ({})", m.name, facts.join(", "))?;
        }
    }
    Ok(())
}

/// Page units without a trailing `.0` for whole numbers.
fn num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{v:.0}")
    } else {
        format!("{}", (v * 100.0).round() / 100.0)
    }
}
