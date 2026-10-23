//! Data for the Desktop-style views and panes that are not plain model / report outlines:
//! the TMDL view (verbatim `.tmdl` files or TMDL generated from the model), the Model view
//! layout (`diagramLayout.json`), the Format pane of a visual (its formatting objects as
//! readable property cards) and the report theme (Design ribbon → Themes gallery).
//!
//! Like [`crate::mapping`], everything here is panic-free and capped: hostile files can
//! neither blow the stack nor bloat the IPC payload.

use pbi_core::{ModelSource, OpenedDocument};
use pbi_model::{Literal, LiteralValue, Model, Report, Visual};
use serde_json::{Map, Value};

use crate::dto::*;
use crate::mapping;

/// Largest TMDL document sent to the TMDL view (bytes); longer ones are cut with a note.
const MAX_TMDL_BYTES: usize = 4 * 1024 * 1024;
/// Largest theme JSON read for the Themes gallery (bytes).
const MAX_THEME_BYTES: u64 = 8 * 1024 * 1024;
/// Theme colors listed in the Themes gallery.
const MAX_THEME_COLORS: usize = 16;
/// Layout tabs / table cards read from `diagramLayout.json`.
const MAX_DIAGRAMS: usize = 64;
const MAX_DIAGRAM_NODES: usize = 4096;
/// Format pane limits: cards, instances per card, properties per instance.
const MAX_FORMAT_CARDS: usize = 256;
const MAX_FORMAT_ENTRIES: usize = 64;
const MAX_FORMAT_PROPERTIES: usize = 256;
/// Longest property value / selector text in the Format pane (characters).
const MAX_VALUE_CHARS: usize = 160;
/// Nesting depth after which formatting expressions are shown as JSON.
const MAX_DEPTH: usize = 12;

fn truncate(text: &str, max_chars: usize) -> String {
    match text.char_indices().nth(max_chars) {
        Some((cut, _)) => format!("{}…", text.get(..cut).unwrap_or_default()),
        None => text.to_owned(),
    }
}

fn cap_bytes(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut cut = max;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!(
        "{}\n// … truncated by DashCraft ({} bytes total)\n",
        text.get(..cut).unwrap_or_default(),
        text.len()
    )
}

fn finite(value: f64) -> f64 {
    if value.is_finite() { value } else { 0.0 }
}

// ---------------------------------------------------------------------------
// TMDL view
// ---------------------------------------------------------------------------

/// Path relative to the TMDL `definition/` folder (`/`-separated).
fn definition_relative(path: &str, definition_dir: &str) -> String {
    let p = path.replace('\\', "/");
    let dir = definition_dir.trim_end_matches('/');
    if !dir.is_empty()
        && let Some(rest) = p.strip_prefix(dir).and_then(|r| r.strip_prefix('/'))
    {
        return rest.to_owned();
    }
    match p.rfind("/definition/") {
        Some(i) => p.get(i + "/definition/".len()..).unwrap_or(&p).to_owned(),
        None => p.strip_prefix("definition/").unwrap_or(&p).to_owned(),
    }
}

/// Name of the first `table` declared in a TMDL document.
fn declared_table(file: &pbi_tmdl::TmdlFile) -> Option<String> {
    file.find("table")
        .find(|n| !n.is_ref())
        .and_then(|n| n.name())
}

/// Sort key: Desktop's TMDL folder order (database, model, then files, then folders).
fn tmdl_rank(path: &str) -> (u8, String) {
    let rank = match path {
        "database.tmdl" => 0,
        "model.tmdl" => 1,
        p if !p.contains('/') => 2,
        p if p.starts_with("tables/") => 3,
        _ => 4,
    };
    (rank, path.to_lowercase())
}

/// The model as TMDL: the opened `.tmdl` files verbatim when the model came from TMDL,
/// otherwise canonical TMDL generated from `model`.
pub(crate) fn tmdl_view(opened: &OpenedDocument, model: &Model) -> TmdlViewDto {
    let (source, mut files) = match &opened.model_source {
        Some(ModelSource::Tmdl {
            definition_dir,
            files,
            ..
        }) if !files.is_empty() => (
            TmdlSourceDto::Files,
            files
                .iter()
                .map(|f| {
                    let text = f.to_string();
                    TmdlFileDto {
                        path: definition_relative(&f.path, definition_dir),
                        text: cap_bytes(
                            text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned(),
                            MAX_TMDL_BYTES,
                        ),
                        table: declared_table(f),
                    }
                })
                .collect::<Vec<_>>(),
        ),
        _ => (
            TmdlSourceDto::Generated,
            pbi_tmdl::write_model(model, None)
                .into_iter()
                .map(|(path, text)| {
                    let table = pbi_tmdl::parse_file(&path, &text)
                        .ok()
                        .as_ref()
                        .and_then(declared_table);
                    TmdlFileDto {
                        path,
                        text: cap_bytes(text, MAX_TMDL_BYTES),
                        table,
                    }
                })
                .collect(),
        ),
    };
    files.sort_by_key(|f| tmdl_rank(&f.path));
    TmdlViewDto { source, files }
}

// ---------------------------------------------------------------------------
// Model view layout (diagramLayout.json)
// ---------------------------------------------------------------------------

fn number(value: Option<&Value>) -> f64 {
    finite(value.and_then(Value::as_f64).unwrap_or(0.0))
}

fn diagram_node(node: &Value) -> Option<DiagramNodeDto> {
    let table = node.get("nodeIndex")?.as_str()?.trim();
    if table.is_empty() {
        return None;
    }
    let location = node.get("location");
    let size = node.get("size");
    let collapsed = ["isCollapsed", "collapsed"]
        .iter()
        .any(|k| node.get(k).and_then(Value::as_bool) == Some(true));
    Some(DiagramNodeDto {
        table: truncate(table, 512),
        x: number(location.and_then(|l| l.get("x"))),
        y: number(location.and_then(|l| l.get("y"))),
        width: number(size.and_then(|s| s.get("width"))).max(0.0),
        height: number(size.and_then(|s| s.get("height"))).max(0.0),
        collapsed,
    })
}

/// Model view layouts from a `diagramLayout.json` value (lenient: unknown shapes are skipped).
pub(crate) fn model_diagram(layout: &Value) -> ModelDiagramDto {
    let diagrams = layout
        .get("diagrams")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .take(MAX_DIAGRAMS)
                .enumerate()
                .map(|(i, d)| DiagramDto {
                    name: d
                        .get("name")
                        .and_then(Value::as_str)
                        .map_or_else(|| format!("Layout {}", i + 1), |n| truncate(n, 256)),
                    nodes: d
                        .get("nodes")
                        .and_then(Value::as_array)
                        .map(|nodes| {
                            nodes
                                .iter()
                                .take(MAX_DIAGRAM_NODES)
                                .filter_map(diagram_node)
                                .collect()
                        })
                        .unwrap_or_default(),
                    zoom: d
                        .get("zoomValue")
                        .and_then(Value::as_f64)
                        .filter(|z| z.is_finite() && *z > 0.0)
                        .unwrap_or(100.0),
                })
                .collect()
        })
        .unwrap_or_default();
    ModelDiagramDto {
        diagrams,
        default_diagram: layout
            .get("defaultDiagram")
            .or_else(|| layout.get("selectedDiagram"))
            .and_then(Value::as_str)
            .map(|s| truncate(s, 256)),
    }
}

// ---------------------------------------------------------------------------
// Format pane
// ---------------------------------------------------------------------------

/// `categoryAxis` → `Category axis`, `fontSize` → `Font size`.
pub(crate) fn label(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_lower = false;
    for c in name.chars() {
        if c == '_' || c == '-' {
            out.push(' ');
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
    }
    let mut chars = out.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => out,
    }
}

fn compact_json(value: &Value) -> String {
    truncate(
        &serde_json::to_string(value).unwrap_or_default(),
        MAX_VALUE_CHARS,
    )
}

/// `Sales[Amount]` from a `{"Expression": {"SourceRef": {"Entity": …}}, "Property": …}` ref.
fn field_ref(value: &Value) -> Option<String> {
    let entity = value
        .pointer("/Expression/SourceRef/Entity")
        .or_else(|| value.pointer("/Expression/SourceRef/Source"))
        .and_then(Value::as_str)?;
    let property = value.get("Property").and_then(Value::as_str)?;
    Some(format!("{entity}[{property}]"))
}

fn literal_text(raw: &str) -> String {
    match Literal::new(raw).value() {
        Ok(LiteralValue::Text(t)) => t,
        Ok(LiteralValue::Boolean(b)) => if b { "On" } else { "Off" }.to_owned(),
        Ok(LiteralValue::Integer(i)) => i.to_string(),
        Ok(LiteralValue::Double(d) | LiteralValue::Decimal(d)) => d.to_string(),
        Ok(LiteralValue::DateTime(t)) => t,
        Ok(LiteralValue::Null) => "(none)".to_owned(),
        Err(_) => raw.to_owned(),
    }
}

fn theme_color(value: &Value) -> String {
    let id = value.get("ColorId").and_then(Value::as_i64).unwrap_or(-1);
    let base = match id {
        0 => "White".to_owned(),
        1 => "Black".to_owned(),
        2..=9 => format!("Theme color {}", id - 1),
        other => format!("Theme color #{other}"),
    };
    let percent = value
        .get("Percent")
        .and_then(Value::as_f64)
        .filter(|p| p.is_finite())
        .unwrap_or(0.0);
    if percent.abs() < 1e-9 {
        base
    } else {
        let pct = (percent.abs() * 100.0).round();
        let how = if percent > 0.0 { "lighter" } else { "darker" };
        format!("{base}, {pct}% {how}")
    }
}

/// Readable form of a formatting expression (`expr`): literals, theme colors, field-based
/// (conditional) formatting; anything else as compact JSON.
fn format_expr(expr: &Value, depth: usize) -> String {
    if depth > MAX_DEPTH {
        return "…".to_owned();
    }
    let Some(obj) = expr.as_object() else {
        return compact_json(expr);
    };
    if let Some(raw) = expr.pointer("/Literal/Value").and_then(Value::as_str) {
        return truncate(&literal_text(raw), MAX_VALUE_CHARS);
    }
    if let Some(color) = obj.get("ThemeDataColor") {
        return theme_color(color);
    }
    for key in ["Measure", "Column"] {
        if let Some(r) = obj.get(key).and_then(field_ref) {
            return format!("fx {r}");
        }
    }
    if let Some(agg) = obj.get("Aggregation") {
        let inner = agg.get("Expression").map_or_else(String::new, |e| {
            format_expr(e, depth + 1)
                .trim_start_matches("fx ")
                .to_owned()
        });
        let function = match agg.get("Function").and_then(Value::as_i64) {
            Some(0) => "Sum",
            Some(1) => "Average",
            Some(2) => "DistinctCount",
            Some(3) => "Min",
            Some(4) => "Max",
            Some(5) => "Count",
            Some(6) => "Median",
            _ => "Aggregate",
        };
        return format!("fx {function}({inner})");
    }
    if obj.contains_key("FillRule") {
        return "fx Gradient (conditional formatting)".to_owned();
    }
    if obj.contains_key("Conditional") {
        return "fx Rules (conditional formatting)".to_owned();
    }
    if let Some(name) = expr
        .pointer("/ResourcePackageItem/ItemName")
        .and_then(Value::as_str)
    {
        return truncate(&format!("Image: {name}"), MAX_VALUE_CHARS);
    }
    compact_json(expr)
}

/// Readable form of a formatting property value.
fn format_value(value: &Value, depth: usize) -> String {
    if depth > MAX_DEPTH {
        return "…".to_owned();
    }
    match value {
        Value::Object(obj) => {
            if let Some(expr) = obj.get("expr") {
                return format_expr(expr, depth + 1);
            }
            if let Some(color) = value.pointer("/solid/color") {
                return format_value(color, depth + 1);
            }
            if obj.len() == 1
                && let Some((key, inner)) = obj.iter().next()
            {
                return format!("{}: {}", label(key), format_value(inner, depth + 1));
            }
            compact_json(value)
        }
        Value::String(s) => truncate(s, MAX_VALUE_CHARS),
        Value::Null => "(none)".to_owned(),
        Value::Bool(b) => if *b { "On" } else { "Off" }.to_owned(),
        other => compact_json(other),
    }
}

fn selector(value: &Value) -> Option<String> {
    let obj = value.as_object().filter(|o| !o.is_empty())?;
    let parts: Vec<String> = obj
        .iter()
        .map(|(k, v)| match v {
            Value::String(s) => format!("{k}: {s}"),
            other => format!("{k}: {}", compact_json(other)),
        })
        .collect();
    Some(truncate(&parts.join(", "), MAX_VALUE_CHARS))
}

fn format_entry(entry: &Value) -> FormatEntryDto {
    let properties = entry
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .iter()
                .take(MAX_FORMAT_PROPERTIES)
                .map(|(name, value)| PropertyDto {
                    label: label(name),
                    value: format_value(value, 0),
                    tooltip: Some(name.clone()),
                    mono: false,
                })
                .collect()
        })
        .unwrap_or_default();
    FormatEntryDto {
        selector: entry.get("selector").and_then(selector),
        properties,
    }
}

/// Format pane cards of a `visual.objects` / `visualContainerObjects` value.
pub(crate) fn format_cards(objects: &Value) -> Vec<FormatCardDto> {
    let Some(map) = objects.as_object() else {
        return Vec::new();
    };
    map.iter()
        .take(MAX_FORMAT_CARDS)
        .map(|(name, entries)| FormatCardDto {
            name: name.clone(),
            label: label(name),
            entries: match entries {
                Value::Array(list) => list
                    .iter()
                    .take(MAX_FORMAT_ENTRIES)
                    .map(format_entry)
                    .collect(),
                other => vec![format_entry(other)],
            },
        })
        .collect()
}

fn layout_property(label: &str, value: String) -> PropertyDto {
    PropertyDto {
        label: label.to_owned(),
        value,
        tooltip: None,
        mono: false,
    }
}

/// The Format pane of a visual.
pub(crate) fn visual_format(v: &Visual) -> VisualFormatDto {
    let p = &v.position;
    let round = |x: f64| format!("{}", finite(x).round());
    let mut layout = vec![
        layout_property("X position", round(p.x)),
        layout_property("Y position", round(p.y)),
        layout_property("Width", round(p.width)),
        layout_property("Height", round(p.height)),
        layout_property("Layer order (z)", round(p.z)),
    ];
    if let Some(tab) = p.tab_order {
        layout.push(layout_property("Tab order", tab.to_string()));
    }
    layout.push(layout_property(
        "Hidden",
        if v.is_hidden { "Yes" } else { "No" }.to_owned(),
    ));
    VisualFormatDto {
        visual: format_cards(&v.objects),
        general: format_cards(&v.container_objects),
        layout,
    }
}

/// The visual with handle `id` in the document's reports.
pub(crate) fn find_visual(reports: &[Report], id: ObjectIdDto) -> Option<&Visual> {
    reports
        .iter()
        .flat_map(|r| r.pages.iter())
        .flat_map(|p| p.visuals.iter())
        .find(|v| mapping::id(v.id) == id)
}

// ---------------------------------------------------------------------------
// Report theme (Themes gallery)
// ---------------------------------------------------------------------------

fn hex_color(value: &Value) -> Option<String> {
    let s = value.as_str()?.trim();
    let digits = s.strip_prefix('#')?;
    (matches!(digits.len(), 3 | 6 | 8) && digits.chars().all(|c| c.is_ascii_hexdigit()))
        .then(|| s.to_ascii_uppercase())
}

fn read_theme(
    opened: &OpenedDocument,
    folder: &str,
    report: &Report,
) -> Option<Map<String, Value>> {
    let resource = report
        .theme
        .custom_theme
        .as_ref()
        .or(report.theme.base_theme.as_ref())?;
    read_theme_resource(opened, folder, report, resource)
}

fn read_theme_resource(
    opened: &OpenedDocument,
    folder: &str,
    report: &Report,
    resource: &pbi_model::ResourceRef,
) -> Option<Map<String, Value>> {
    let relative = report.static_resource_path(resource)?;
    let path = if folder.is_empty() {
        relative
    } else {
        format!("{}/{relative}", folder.trim_end_matches('/'))
    };
    let container = opened.container.as_ref();
    if container
        .entry(&path)
        .is_none_or(|e| e.size > MAX_THEME_BYTES)
    {
        return None;
    }
    let text = container.read_text(&path).ok()?.text;
    match serde_json::from_str(&text).ok()? {
        Value::Object(map) => Some(map),
        _ => None,
    }
}

/// Theme of report `index` (base / custom theme names, colors of the effective theme file).
pub(crate) fn report_theme(opened: &OpenedDocument, index: usize) -> ReportThemeDto {
    let Some(report) = opened.document.reports.get(index) else {
        return ReportThemeDto::default();
    };
    let mut dto = ReportThemeDto {
        base_theme: report.theme.base_theme.as_ref().map(|t| t.name.clone()),
        custom_theme: report.theme.custom_theme.as_ref().map(|t| t.name.clone()),
        ..ReportThemeDto::default()
    };
    let folder = opened
        .report_sources
        .get(index)
        .map(pbi_core::ReportSource::folder)
        .unwrap_or_default();
    if let Some(theme) = read_theme(opened, folder, report) {
        dto.name = theme
            .get("name")
            .and_then(Value::as_str)
            .map(|n| truncate(n, 256));
        dto.data_colors = theme
            .get("dataColors")
            .and_then(Value::as_array)
            .map(|c| {
                c.iter()
                    .filter_map(hex_color)
                    .take(MAX_THEME_COLORS)
                    .collect()
            })
            .unwrap_or_default();
        dto.background = theme.get("background").and_then(hex_color);
        dto.foreground = theme.get("foreground").and_then(hex_color);
        dto.table_accent = theme.get("tableAccent").and_then(hex_color);
    }
    dto
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn labels_split_camel_case() {
        assert_eq!(label("categoryAxis"), "Category axis");
        assert_eq!(label("fontSize"), "Font size");
        assert_eq!(label("show"), "Show");
        assert_eq!(label("visualHeader"), "Visual header");
        assert_eq!(label(""), "");
    }

    #[test]
    fn format_cards_read_literals_colors_and_fields() {
        let objects = json!({
            "title": [{ "properties": {
                "show": { "expr": { "Literal": { "Value": "true" } } },
                "text": { "expr": { "Literal": { "Value": "'Total apps'" } } },
                "fontSize": { "expr": { "Literal": { "Value": "14D" } } },
                "fontColor": { "solid": { "color": { "expr": {
                    "ThemeDataColor": { "ColorId": 0, "Percent": -0.6 } } } } }
            }}],
            "dataPoint": [{
                "properties": { "fill": { "solid": { "color": { "expr": {
                    "Measure": { "Expression": { "SourceRef": { "Entity": "Apps" } },
                                 "Property": "Color" } } } } } },
                "selector": { "metadata": "Sum(Apps.Count)" }
            }]
        });
        let cards = format_cards(&objects);
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].label, "Title");
        let values: Vec<(&str, &str)> = cards[0].entries[0]
            .properties
            .iter()
            .map(|p| (p.label.as_str(), p.value.as_str()))
            .collect();
        assert_eq!(
            values,
            [
                ("Show", "On"),
                ("Text", "Total apps"),
                ("Font size", "14"),
                ("Font color", "White, 60% darker"),
            ]
        );
        let point = &cards[1].entries[0];
        assert_eq!(point.selector.as_deref(), Some("metadata: Sum(Apps.Count)"));
        assert_eq!(point.properties[0].value, "fx Apps[Color]");
        assert!(format_cards(&Value::Null).is_empty());
    }

    #[test]
    fn diagram_layouts_are_read_leniently() {
        let layout = json!({
            "version": "1.1.0",
            "diagrams": [{
                "name": "All tables",
                "zoomValue": 80,
                "nodes": [
                    { "location": { "x": 10, "y": 20.5 }, "nodeIndex": "Orders",
                      "size": { "height": 300, "width": 234 } },
                    { "nodeIndex": "" },
                    { "location": "bad", "nodeIndex": "Web Sessions" }
                ]
            }, "not a diagram"],
            "defaultDiagram": "All tables"
        });
        let dto = model_diagram(&layout);
        assert_eq!(dto.default_diagram.as_deref(), Some("All tables"));
        assert_eq!(dto.diagrams.len(), 2);
        let all = &dto.diagrams[0];
        assert_eq!((all.name.as_str(), all.zoom), ("All tables", 80.0));
        assert_eq!(all.nodes.len(), 2);
        assert_eq!((all.nodes[0].x, all.nodes[0].y), (10.0, 20.5));
        assert_eq!(all.nodes[0].width, 234.0);
        assert_eq!(
            (all.nodes[1].table.as_str(), all.nodes[1].x),
            ("Web Sessions", 0.0)
        );
        assert_eq!(dto.diagrams[1].name, "Layout 2");
        assert!(model_diagram(&json!([])).diagrams.is_empty());
    }

    #[test]
    fn tmdl_paths_are_relative_to_the_definition_folder() {
        assert_eq!(
            definition_relative(
                "M.SemanticModel/definition/tables/A.tmdl",
                "M.SemanticModel/definition"
            ),
            "tables/A.tmdl"
        );
        assert_eq!(
            definition_relative("x/definition/model.tmdl", ""),
            "model.tmdl"
        );
        assert_eq!(definition_relative("model.tmdl", ""), "model.tmdl");
        let mut paths = [
            "tables/B.tmdl",
            "relationships.tmdl",
            "model.tmdl",
            "database.tmdl",
            "roles/R.tmdl",
        ];
        paths.sort_by_key(|p| tmdl_rank(p));
        assert_eq!(
            paths,
            [
                "database.tmdl",
                "model.tmdl",
                "relationships.tmdl",
                "tables/B.tmdl",
                "roles/R.tmdl"
            ]
        );
    }
}
