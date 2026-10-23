//! Markdown data dictionary ([`model_to_markdown`]).

use pbi_model::Model;
use serde::{Deserialize, Serialize};

use super::model_doc::{
    ColumnDoc, ExpressionDoc, MeasureDoc, ModelDoc, ModelDocOptions, ParameterDoc, PartitionDoc,
    RelationshipDoc, RoleDoc, TableDoc, model_doc, text,
};

/// Options of [`model_to_markdown`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct MarkdownOptions {
    /// Include DAX and M code blocks (default `true`).
    pub include_expressions: bool,
    /// Include hidden tables, columns, measures and hierarchies (default
    /// `true`).
    pub include_hidden: bool,
    /// Document title (default: the model name). Callers typically pass the
    /// project or file name (`AcmeOutfitters`), as TMDL models are usually
    /// named `Model`.
    pub title: Option<String>,
}

impl Default for MarkdownOptions {
    fn default() -> Self {
        Self {
            include_expressions: true,
            include_hidden: true,
            title: None,
        }
    }
}

/// Renders `model` as a Markdown data dictionary: title, summary counts,
/// the relationship table and a table index, then per table its description
/// (TMDL `///`), a columns table (Name | Type | Description | Source),
/// measures (format, description, DAX in ` ```dax ` blocks), calculated
/// columns, hierarchies and its source (M in ` ```powerquery ` blocks, DAX of
/// calculated tables); then parameters, shared expressions and roles.
///
/// The output is deterministic (model order, no timestamps). Text is
/// escaped: Markdown punctuation is backslash-escaped, `|` in table cells
/// becomes `\|` and line breaks `<br>`; code fences are longer than any
/// backtick run in the code.
#[must_use]
pub fn model_to_markdown(model: &Model, options: &MarkdownOptions) -> String {
    let doc = model_doc(
        model,
        &ModelDocOptions {
            include_expressions: options.include_expressions,
            include_hidden: options.include_hidden,
        },
    );
    let title = text(options.title.as_deref()).unwrap_or_else(|| doc.name.clone());
    let mut md = Md::default();
    write_header(&mut md, &doc, &title);
    write_relationships(&mut md, &doc.relationships);
    write_tables(&mut md, &doc);
    write_parameters(&mut md, &doc.parameters, doc.includes_expressions);
    write_expressions(&mut md, &doc.expressions, doc.includes_expressions);
    write_roles(&mut md, &doc.roles, doc.includes_expressions);
    md.finish()
}

/// Output buffer: blocks separated by one blank line, ending with one `\n`.
#[derive(Default)]
struct Md {
    out: String,
}

impl Md {
    /// Starts a new block (a blank line before it, except at the start).
    fn block(&mut self) {
        if !self.out.is_empty() && !self.out.ends_with("\n\n") {
            self.out.push('\n');
        }
    }

    fn line(&mut self, line: impl AsRef<str>) {
        self.out.push_str(line.as_ref());
        self.out.push('\n');
    }

    fn heading(&mut self, level: usize, title: &str) {
        self.block();
        self.line(format!("{} {}", "#".repeat(level), heading_text(title)));
    }

    fn paragraph(&mut self, text: &str) {
        self.block();
        self.line(paragraph(text));
    }

    /// A paragraph of already escaped Markdown.
    fn paragraph_raw(&mut self, markdown: &str) {
        self.block();
        self.line(markdown);
    }

    fn table<R: AsRef<[String]>>(&mut self, header: &[&str], right: &[usize], rows: &[R]) {
        self.block();
        self.line(format!("| {} |", header.join(" | ")));
        let align: Vec<&str> = (0..header.len())
            .map(|i| if right.contains(&i) { "---:" } else { "---" })
            .collect();
        self.line(format!("|{}|", align.join("|")));
        for row in rows {
            self.line(format!("| {} |", row.as_ref().join(" | ")));
        }
    }

    fn code(&mut self, language: &str, code: &str) {
        self.block();
        let fence = "`".repeat(longest_backtick_run(code).max(2) + 1);
        self.line(format!("{fence}{language}"));
        for line in code.trim_end().lines() {
            self.line(line.trim_end());
        }
        self.line(fence);
    }

    fn finish(mut self) -> String {
        while self.out.ends_with("\n\n") {
            self.out.pop();
        }
        self.out
    }
}

fn write_header(md: &mut Md, doc: &ModelDoc, title: &str) {
    md.heading(1, title);
    let mut about = format!(
        "Data dictionary of the semantic model **{}**",
        inline(&doc.name)
    );
    let details: Vec<String> = [
        doc.culture
            .as_ref()
            .map(|c| format!("culture {}", inline(c))),
        doc.compatibility_level
            .map(|l| format!("compatibility level {l}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if !details.is_empty() {
        about.push_str(&format!(" ({})", details.join(", ")));
    }
    about.push('.');
    if !doc.includes_expressions {
        about.push_str(" DAX and M expressions are omitted.");
    }
    if !doc.includes_hidden {
        about.push_str(" Hidden objects are omitted.");
    }
    md.block();
    md.line(about);
    if let Some(d) = &doc.description {
        md.paragraph(d);
    }

    md.heading(2, "Summary");
    let s = &doc.summary;
    let rows = [
        ("Tables", s.tables),
        ("Columns", s.columns),
        ("Calculated columns", s.calculated_columns),
        ("Calculated tables", s.calculated_tables),
        ("Measures", s.measures),
        ("Relationships", s.relationships),
        ("Parameters", s.parameters),
        ("Shared expressions", s.expressions),
        ("Roles", s.roles),
    ]
    .map(|(what, n)| [what.to_owned(), n.to_string()]);
    md.table(&["Object", "Count"], &[1], &rows);
}

fn write_relationships(md: &mut Md, relationships: &[RelationshipDoc]) {
    md.heading(2, "Relationships");
    if relationships.is_empty() {
        md.paragraph("No relationships.");
        return;
    }
    let rows: Vec<[String; 4]> = relationships
        .iter()
        .map(|r| {
            [
                cell(&format!(
                    "{}[{}] → {}[{}]",
                    r.from.table, r.from.column, r.to.table, r.to.column
                )),
                cell(&format!("{}-to-{}", r.from_cardinality, r.to_cardinality)),
                cell(cross_filter(&r.cross_filtering_behavior)),
                yes_no(r.active),
            ]
        })
        .collect();
    md.table(
        &["From → To", "Cardinality", "Cross-filter", "Active"],
        &[],
        &rows,
    );
}

fn write_tables(md: &mut Md, doc: &ModelDoc) {
    md.heading(2, "Tables");
    if doc.tables.is_empty() {
        md.paragraph("No tables.");
        return;
    }
    let rows: Vec<[String; 5]> = doc
        .tables
        .iter()
        .map(|t| {
            [
                cell(&t.name),
                cell(&table_kind_label(t)),
                t.columns.len().to_string(),
                t.measures.len().to_string(),
                cell(t.description.as_deref().unwrap_or_default()),
            ]
        })
        .collect();
    md.table(
        &["Table", "Kind", "Columns", "Measures", "Description"],
        &[2, 3],
        &rows,
    );
    for t in &doc.tables {
        write_table(md, t, doc.includes_expressions);
    }
}

fn table_kind_label(t: &TableDoc) -> String {
    let kind = match t.kind.as_str() {
        "regular" => "table",
        "calculatedTable" => "calculated table",
        "calculationGroup" => "calculation group",
        "remote" => "remote table",
        other => other,
    };
    if t.hidden {
        format!("{kind} (hidden)")
    } else {
        kind.to_owned()
    }
}

fn write_table(md: &mut Md, t: &TableDoc, expressions: bool) {
    md.heading(3, &t.name);
    if let Some(d) = &t.description {
        md.paragraph(d);
    }
    let mut facts = vec![format!("Kind: {}", inline(&table_kind_label(t)))];
    if let Some(c) = &t.data_category {
        facts.push(format!("data category: {}", inline(c)));
    }
    md.paragraph_raw(&format!("{}.", facts.join(" · ")));

    if !t.columns.is_empty() {
        md.heading(4, "Columns");
        let rows: Vec<[String; 4]> = t.columns.iter().map(column_row).collect();
        md.table(&["Name", "Type", "Description", "Source"], &[], &rows);
    }
    if !t.measures.is_empty() {
        write_measures(md, &t.measures, expressions);
    }
    let calculated: Vec<&ColumnDoc> = t
        .columns
        .iter()
        .filter(|c| c.kind == "calculated")
        .collect();
    if expressions && !calculated.is_empty() {
        md.heading(4, "Calculated columns");
        for c in calculated {
            md.heading(5, &c.name);
            if let Some(d) = &c.description {
                md.paragraph(d);
            }
            if let Some(e) = &c.expression {
                md.code("dax", e);
            }
        }
    }
    if !t.hierarchies.is_empty() {
        md.heading(4, "Hierarchies");
        let rows: Vec<[String; 3]> = t
            .hierarchies
            .iter()
            .map(|h| {
                let levels: Vec<String> = h
                    .levels
                    .iter()
                    .map(|l| {
                        if l.name == l.column {
                            l.name.clone()
                        } else {
                            format!("{} ({})", l.name, l.column)
                        }
                    })
                    .collect();
                [
                    cell(&hidden_name(&h.name, h.hidden, false)),
                    cell(&levels.join(" → ")),
                    cell(h.description.as_deref().unwrap_or_default()),
                ]
            })
            .collect();
        md.table(&["Hierarchy", "Levels", "Description"], &[], &rows);
    }
    if !t.partitions.is_empty() {
        md.heading(4, "Source");
        for p in &t.partitions {
            write_partition(md, p);
        }
    }
}

fn hidden_name(name: &str, hidden: bool, key: bool) -> String {
    let flags: Vec<&str> = [hidden.then_some("hidden"), key.then_some("key")]
        .into_iter()
        .flatten()
        .collect();
    if flags.is_empty() {
        name.to_owned()
    } else {
        format!("{name} ({})", flags.join(", "))
    }
}

fn column_row(c: &ColumnDoc) -> [String; 4] {
    let source = match c.kind.as_str() {
        "calculated" => "calculated (DAX)".to_owned(),
        _ => c.source_column.clone().unwrap_or_default(),
    };
    let mut kind = c.data_type.clone();
    if let Some(f) = &c.format_string {
        kind = format!("{kind}, format {f}");
    }
    [
        cell(&hidden_name(&c.name, c.hidden, c.key)),
        cell(&kind),
        cell(c.description.as_deref().unwrap_or_default()),
        cell(&source),
    ]
}

fn write_measures(md: &mut Md, measures: &[MeasureDoc], expressions: bool) {
    md.heading(4, "Measures");
    let folders = measures.iter().any(|m| m.display_folder.is_some());
    let rows: Vec<Vec<String>> = measures
        .iter()
        .map(|m| {
            let mut row = vec![
                cell(&hidden_name(&m.name, m.hidden, false)),
                m.format_string
                    .as_deref()
                    .map(code_cell)
                    .unwrap_or_default(),
            ];
            if folders {
                row.push(cell(m.display_folder.as_deref().unwrap_or_default()));
            }
            row.push(cell(m.description.as_deref().unwrap_or_default()));
            row
        })
        .collect();
    let header: &[&str] = if folders {
        &["Measure", "Format", "Folder", "Description"]
    } else {
        &["Measure", "Format", "Description"]
    };
    md.table(header, &[], &rows);
    if expressions {
        for m in measures {
            if let Some(e) = &m.expression {
                md.heading(5, &m.name);
                if let Some(d) = &m.description {
                    md.paragraph(d);
                }
                md.code("dax", e);
            }
        }
    }
}

fn write_partition(md: &mut Md, p: &PartitionDoc) {
    let what = match (p.kind.as_str(), p.language.as_deref()) {
        ("m", _) => "Power Query (M) partition".to_owned(),
        ("calculated", _) => "Calculated table (DAX) partition".to_owned(),
        ("query", _) => "Native query partition".to_owned(),
        ("entity", _) => "Entity partition".to_owned(),
        (other, _) => format!("{} partition", inline(other)),
    };
    let mut line = format!("{what} {}, {} mode", code_span(&p.name), inline(&p.mode));
    if let Some(e) = &p.entity_name {
        line.push_str(&format!(", entity {}", code_span(e)));
    }
    if let Some(s) = &p.expression_source {
        line.push_str(&format!(", from expression {}", code_span(s)));
    }
    if let Some(g) = &p.query_group {
        line.push_str(&format!(", query group {}", code_span(g)));
    }
    line.push('.');
    md.paragraph_raw(&line);
    if let Some(d) = &p.description {
        md.paragraph(d);
    }
    if let Some(e) = &p.expression {
        let language = match p.language.as_deref() {
            Some("m") => "powerquery",
            Some(other) => other,
            None => "",
        };
        md.code(language, e);
    }
}

fn write_parameters(md: &mut Md, parameters: &[ParameterDoc], expressions: bool) {
    if parameters.is_empty() {
        return;
    }
    md.heading(2, "Parameters");
    if expressions {
        let rows: Vec<[String; 5]> = parameters
            .iter()
            .map(|p| {
                [
                    cell(&p.name),
                    cell(p.parameter_type.as_deref().unwrap_or_default()),
                    yes_no(p.required),
                    p.value.as_deref().map(code_cell).unwrap_or_default(),
                    cell(p.description.as_deref().unwrap_or_default()),
                ]
            })
            .collect();
        md.table(
            &["Parameter", "Type", "Required", "Value", "Description"],
            &[],
            &rows,
        );
    } else {
        let rows: Vec<[String; 4]> = parameters
            .iter()
            .map(|p| {
                [
                    cell(&p.name),
                    cell(p.parameter_type.as_deref().unwrap_or_default()),
                    yes_no(p.required),
                    cell(p.description.as_deref().unwrap_or_default()),
                ]
            })
            .collect();
        md.table(
            &["Parameter", "Type", "Required", "Description"],
            &[],
            &rows,
        );
    }
}

fn write_expressions(md: &mut Md, expressions: &[ExpressionDoc], include: bool) {
    if expressions.is_empty() {
        return;
    }
    md.heading(2, "Shared expressions");
    for e in expressions {
        md.heading(3, &e.name);
        let language = if e.kind == "dax" {
            "DAX"
        } else {
            "Power Query (M)"
        };
        let mut line = format!("{language} expression");
        if let Some(g) = &e.query_group {
            line.push_str(&format!(", query group {}", code_span(g)));
        }
        line.push('.');
        md.paragraph_raw(&line);
        if let Some(d) = &e.description {
            md.paragraph(d);
        }
        if let (true, Some(x)) = (include, &e.expression) {
            md.code(if e.kind == "dax" { "dax" } else { "powerquery" }, x);
        }
    }
}

fn write_roles(md: &mut Md, roles: &[RoleDoc], expressions: bool) {
    if roles.is_empty() {
        return;
    }
    md.heading(2, "Roles");
    for r in roles {
        md.heading(3, &r.name);
        if let Some(d) = &r.description {
            md.paragraph(d);
        }
        if let Some(p) = &r.model_permission {
            md.paragraph_raw(&format!("Model permission: {}.", inline(p)));
        }
        for p in &r.table_permissions {
            md.paragraph_raw(&format!("Row filter on {}:", code_span(&p.table)));
            match (&p.filter_expression, expressions) {
                (Some(f), _) => md.code("dax", f),
                (None, true) => md.paragraph("(none)"),
                (None, false) => md.paragraph("(omitted)"),
            }
        }
    }
}

fn cross_filter(behavior: &str) -> &str {
    match behavior {
        "oneDirection" => "single",
        "bothDirections" => "both directions",
        other => other,
    }
}

fn yes_no(b: bool) -> String {
    if b { "yes" } else { "no" }.to_owned()
}

// ---- escaping ----------------------------------------------------------------

/// Escapes inline text: `\`, `` ` ``, `*`, `<`, `>`, `|`, `[`, `]` and `_`
/// where it could start or end emphasis (not between two alphanumerics).
fn inline(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    for (i, &c) in chars.iter().enumerate() {
        let escape = match c {
            '\\' | '`' | '*' | '<' | '>' | '|' => true,
            '_' => {
                let before = i
                    .checked_sub(1)
                    .and_then(|j| chars.get(j))
                    .is_some_and(|c| c.is_alphanumeric());
                let after = chars.get(i + 1).is_some_and(|c| c.is_alphanumeric());
                !(before && after)
            }
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Heading text: inline escaping plus `#` (a trailing `#` run would be
/// read as a closing sequence).
fn heading_text(text: &str) -> String {
    inline(&one_line(text)).replace('#', "\\#")
}

/// A table cell: inline escaping, line breaks as `<br>`.
fn cell(text: &str) -> String {
    let lines: Vec<String> = text
        .trim()
        .lines()
        .map(|l| inline(l.trim()))
        .filter(|l| !l.is_empty())
        .collect();
    lines.join("<br>")
}

/// A paragraph: inline escaping per line; a line that would start a block
/// (heading, list, thematic break, setext underline, fence) gets its marker
/// escaped; indentation is dropped (it would make a code block).
fn paragraph(text: &str) -> String {
    let lines: Vec<String> = text
        .trim()
        .lines()
        .map(|l| escape_block_start(inline(l.trim())))
        .collect();
    lines.join("\n")
}

fn escape_block_start(line: String) -> String {
    if line.starts_with(['#', '-', '+', '=', '~']) {
        return format!("\\{line}");
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    match (line.get(..digits), line.get(digits..)) {
        (Some(number), Some(rest)) if (1..=9).contains(&digits) && rest.starts_with(['.', ')']) => {
            format!("{number}\\{rest}")
        }
        _ => line,
    }
}

/// Collapses whitespace (including line breaks) to single spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn longest_backtick_run(text: &str) -> usize {
    let (mut longest, mut run) = (0, 0);
    for c in text.chars() {
        if c == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// An inline code span (on one line), with a delimiter longer than any
/// backtick run inside.
fn code_span(code: &str) -> String {
    let code = one_line(code);
    let fence = "`".repeat(longest_backtick_run(&code) + 1);
    let pad = if code.starts_with('`') || code.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{pad}{code}{pad}{fence}")
}

/// A code span inside a table cell (`|` escaped, as GFM requires).
fn code_cell(code: &str) -> String {
    code_span(code).replace('|', "\\|")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_is_escaped_for_markdown() {
        assert_eq!(inline("a|b *c* <d> \\ `e`"), r"a\|b \*c\* \<d\> \\ \`e\`");
        assert_eq!(inline("order_id _x_"), r"order_id \_x\_");
        assert_eq!(cell(" one | two\r\nthree \n"), r"one \| two<br>three");
        assert_eq!(heading_text("Total #\nOrders"), r"Total \# Orders");
        assert_eq!(
            paragraph("# not a heading\n- not a list\n2. no"),
            "\\# not a heading\n\\- not a list\n2\\. no"
        );
        assert_eq!(paragraph("Version 2.0 is fine"), "Version 2.0 is fine");
    }

    #[test]
    fn code_spans_and_fences_outgrow_backticks() {
        assert_eq!(code_span("0.0%"), "`0.0%`");
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("`x"), "`` `x ``");
        assert_eq!(code_cell("a|b"), r"`a\|b`");
        let mut md = Md::default();
        md.code("dax", "x ```` y");
        assert_eq!(md.finish(), "`````dax\nx ```` y\n`````\n");
    }
}
