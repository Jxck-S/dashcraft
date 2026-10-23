//! The tool set of an MCP server: the agent-safe commands of a registry
//! (after `--allow`/`--deny`), their names, MCP definitions and call
//! results. No per-tool code: everything derives from [`CommandSpec`].

use std::collections::HashMap;
use std::fmt::Write as _;

use serde_json::{Map, Value, json};

use super::protocol::at_least;
use crate::command::{Category, CommandMeta, CommandSpec};
use crate::error::OpError;
use crate::registry::Registry;

/// Prefix of the `_meta` keys DashCraft adds to tool definitions.
pub const META_PREFIX: &str = "dashcraft";

/// Field of render results holding a base64 PNG; it becomes `image`
/// content.
const PNG_FIELD: &str = "pngBase64";

/// Parameters worth pointing at when a result is too large.
const PAGING_PARAMS: &[&str] = &["limit", "offset", "maxRows", "filter", "outputPath"];

/// How tool names derive from command ids.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolNames {
    /// `doc.open` → `doc_open`: matches `^[a-zA-Z0-9_-]{1,64}$`, which many
    /// clients require.
    #[default]
    Underscore,
    /// The command ids as they are (`doc.open`).
    Dotted,
}

impl ToolNames {
    /// The tool name of command `id`.
    #[must_use]
    pub fn name_of(self, id: &str) -> String {
        match self {
            ToolNames::Underscore => id.replace('.', "_"),
            ToolNames::Dotted => id.to_owned(),
        }
    }

    /// `true` when `name` is a valid tool name in this style.
    #[must_use]
    pub fn is_valid(self, name: &str) -> bool {
        let (max, dot) = match self {
            ToolNames::Underscore => (64, false),
            ToolNames::Dotted => (128, true),
        };
        !name.is_empty()
            && name.len() <= max
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || (dot && c == '.'))
    }
}

/// Which agent-safe commands become tools. Entries are category names
/// (`canvas`), command ids (`doc.open`) or tool names (`doc_open`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolFilter {
    /// Only these (empty: every agent-safe command).
    pub allow: Vec<String>,
    /// Never these (applied after `allow`).
    pub deny: Vec<String>,
}

fn matches(entry: &str, meta: &CommandMeta) -> bool {
    entry == meta.id
        || entry == ToolNames::Underscore.name_of(meta.id)
        || entry.eq_ignore_ascii_case(meta.category.as_str())
}

fn is_category(entry: &str) -> bool {
    Category::ALL
        .iter()
        .any(|c| entry.eq_ignore_ascii_case(c.as_str()))
}

/// Behaviour hints of a command (MCP `ToolAnnotations`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hints {
    /// Changes nothing: no document state, no files.
    pub read_only: bool,
    /// May overwrite files (only commands that write files).
    pub destructive: bool,
    /// Repeating the call with the same arguments has no further effect.
    pub idempotent: bool,
}

impl Hints {
    /// Hints from the command flags: read-only unless it mutates a
    /// document or writes files; destructive only when it writes files
    /// (document mutations are in-memory and session-only); idempotent
    /// unless it mutates a document (clicks, scrolls, CSV loads).
    #[must_use]
    pub fn of(meta: &CommandMeta) -> Self {
        Self {
            read_only: !meta.mutates_document && !meta.writes_files,
            destructive: meta.writes_files,
            idempotent: !meta.mutates_document,
        }
    }

    fn union(self, other: Self) -> Self {
        Self {
            read_only: self.read_only && other.read_only,
            destructive: self.destructive || other.destructive,
            idempotent: self.idempotent && other.idempotent,
        }
    }
}

/// The tools of one server. Cheap to share.
#[derive(Debug, Clone)]
pub struct ToolCatalog {
    /// The exposed commands only: `commands.list`/`commands.batch` see the
    /// same set.
    registry: Registry,
    style: ToolNames,
    /// `(tool name, command id)` in catalog order.
    tools: Vec<(String, &'static str)>,
    by_name: HashMap<String, &'static str>,
    by_id: HashMap<&'static str, usize>,
    composite_hints: Hints,
}

impl ToolCatalog {
    /// The agent-safe commands of `registry`, filtered by `filter`, named
    /// by `style`.
    ///
    /// # Errors
    /// `invalidParams` for a filter entry that matches no agent-safe
    /// command or category, or when two commands map to the same tool name
    /// (or a name is invalid).
    pub fn new(
        registry: &Registry,
        filter: &ToolFilter,
        style: ToolNames,
    ) -> Result<Self, OpError> {
        let agent = registry.agent_catalog();
        for (flag, entries) in [("--allow", &filter.allow), ("--deny", &filter.deny)] {
            for entry in entries {
                if is_category(entry) || agent.list().iter().any(|s| matches(entry, &s.meta)) {
                    continue;
                }
                let why = if registry.list().iter().any(|s| matches(entry, &s.meta)) {
                    "is not agent-safe and is never exposed".to_owned()
                } else {
                    let categories: Vec<&str> = Category::ALL.iter().map(|c| c.as_str()).collect();
                    format!(
                        "matches no command or category (categories: {})",
                        categories.join(", ")
                    )
                };
                return Err(OpError::invalid_params(format!("{flag} {entry}: {why}")));
            }
        }
        let exposed = agent.filter(|m| {
            (filter.allow.is_empty() || filter.allow.iter().any(|e| matches(e, m)))
                && !filter.deny.iter().any(|e| matches(e, m))
        });
        let mut tools = Vec::with_capacity(exposed.len());
        let mut by_name: HashMap<String, &'static str> = HashMap::new();
        let mut by_id = HashMap::new();
        let mut composite_hints = Hints {
            read_only: true,
            destructive: false,
            idempotent: true,
        };
        for spec in exposed.list() {
            let id = spec.meta.id;
            if by_id.contains_key(id) {
                continue;
            }
            let name = style.name_of(id);
            if !style.is_valid(&name) {
                return Err(OpError::invalid_params(format!(
                    "command {id} has no valid tool name ({name})"
                )));
            }
            if let Some(other) = by_name.insert(name.clone(), id) {
                return Err(OpError::invalid_params(format!(
                    "commands {other} and {id} both map to the tool name {name}; \
                     use dotted tool names or rename one"
                )));
            }
            by_id.insert(id, tools.len());
            tools.push((name, id));
            composite_hints = composite_hints.union(Hints::of(&spec.meta));
        }
        Ok(Self {
            registry: exposed,
            style,
            tools,
            by_name,
            by_id,
            composite_hints,
        })
    }

    /// The exposed commands (what `tools/call` executes).
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// The naming style.
    #[must_use]
    pub fn style(&self) -> ToolNames {
        self.style
    }

    /// Number of tools.
    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// `true` without tools.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// `(tool name, command id)` pairs in catalog order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &'static str)> {
        self.tools.iter().map(|(n, id)| (n.as_str(), *id))
    }

    /// The command behind tool `name`.
    #[must_use]
    pub fn id_of(&self, name: &str) -> Option<&'static str> {
        self.by_name.get(name).copied()
    }

    /// The tool name of command `id` (if exposed).
    #[must_use]
    pub fn name_of(&self, id: &str) -> Option<&str> {
        self.by_id
            .get(id)
            .and_then(|&i| self.tools.get(i))
            .map(|(n, _)| n.as_str())
    }

    /// The hints of a command. Commands that
    /// [invoke other commands](CommandMeta::invokes_other_commands) get the
    /// union of the exposed catalog's hints, since a step may be any tool.
    #[must_use]
    pub fn hints(&self, meta: &CommandMeta) -> Hints {
        if meta.invokes_other_commands {
            self.composite_hints
        } else {
            Hints::of(meta)
        }
    }

    /// The MCP `Tool` of `spec`, shaped for protocol `version`.
    #[must_use]
    pub fn definition(&self, name: &str, spec: &CommandSpec, version: &str) -> Value {
        let meta = &spec.meta;
        let hints = self.hints(meta);
        let mut description = meta.summary.to_owned();
        if let Some((_, params)) = meta.examples.first() {
            let _ = write!(description, " Example arguments: {params}");
        }
        let mut tool = Map::new();
        tool.insert("name".into(), name.into());
        if at_least(version, "2025-06-18") {
            tool.insert("title".into(), meta.title.into());
        }
        tool.insert("description".into(), description.into());
        tool.insert("inputSchema".into(), object_schema(&spec.params_schema));
        if at_least(version, "2025-06-18") && is_object_schema(&spec.output_schema) {
            tool.insert("outputSchema".into(), spec.output_schema.clone());
        }
        if at_least(version, "2025-03-26") {
            tool.insert(
                "annotations".into(),
                json!({
                    "title": meta.title,
                    "readOnlyHint": hints.read_only,
                    "destructiveHint": hints.destructive,
                    "idempotentHint": hints.idempotent,
                    "openWorldHint": false,
                }),
            );
        }
        tool.insert(
            "_meta".into(),
            json!({
                format!("{META_PREFIX}/commandId"): meta.id,
                format!("{META_PREFIX}/category"): meta.category.as_str(),
            }),
        );
        Value::Object(tool)
    }

    /// One page of `tools/list`: tools from `cursor` (an offset; `None` =
    /// the start), at most `page_size` (0 = all).
    ///
    /// # Errors
    /// A message for an invalid cursor.
    pub fn list(
        &self,
        cursor: Option<&str>,
        page_size: usize,
        version: &str,
    ) -> Result<Value, String> {
        let start = match cursor {
            None => 0,
            Some(c) => c
                .parse::<usize>()
                .ok()
                .filter(|n| *n <= self.tools.len())
                .ok_or_else(|| format!("invalid cursor {c:?}"))?,
        };
        let end = if page_size == 0 {
            self.tools.len()
        } else {
            start.saturating_add(page_size).min(self.tools.len())
        };
        let tools: Vec<Value> = self
            .tools
            .get(start..end)
            .unwrap_or_default()
            .iter()
            .filter_map(|(name, id)| {
                self.registry
                    .describe(id)
                    .map(|spec| self.definition(name, spec, version))
            })
            .collect();
        let mut out = json!({ "tools": tools });
        if end < self.tools.len()
            && let Value::Object(map) = &mut out
        {
            map.insert("nextCursor".into(), end.to_string().into());
        }
        Ok(out)
    }

    /// The `CallToolResult` of a successful command run: the output as
    /// JSON text (plus `structuredContent` when the tool has an output
    /// schema), PNG renders as `image` content (not duplicated in the
    /// text), and text over `max_text_bytes` cut with a note (then marked
    /// `isError` so clients that validate `structuredContent` accept it).
    #[must_use]
    pub fn call_result(
        &self,
        id: &str,
        mut output: Value,
        version: &str,
        max_text_bytes: usize,
    ) -> Value {
        let mut images = Vec::new();
        extract_pngs(&mut output, &mut images);
        let text = output.to_string();
        let spec = self.registry.describe(id);
        let mut content = Vec::with_capacity(1 + images.len());
        if text.len() > max_text_bytes {
            let mut cut = max_text_bytes;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            let mut note = format!(
                "{}\n\n[truncated: the result is {} bytes, over the {} byte limit of this server. \
                 Ask for less",
                text.get(..cut).unwrap_or_default(),
                text.len(),
                max_text_bytes
            );
            let params: Vec<&str> = spec
                .and_then(|s| s.params_schema.get("properties"))
                .and_then(Value::as_object)
                .map(|props| {
                    PAGING_PARAMS
                        .iter()
                        .copied()
                        .filter(|p| props.contains_key(*p))
                        .collect()
                })
                .unwrap_or_default();
            if params.is_empty() {
                note.push_str(" (narrower parameters).]");
            } else {
                let _ = write!(note, " with the parameters {}.]", params.join(", "));
            }
            content.push(json!({ "type": "text", "text": note }));
            content.extend(images);
            return json!({ "content": content, "isError": true });
        }
        content.push(json!({ "type": "text", "text": text }));
        content.extend(images);
        let mut result = json!({ "content": content, "isError": false });
        let structured = at_least(version, "2025-06-18")
            && spec.is_some_and(|s| is_object_schema(&s.output_schema));
        if structured && let Value::Object(map) = &mut result {
            map.insert("structuredContent".into(), output);
        }
        result
    }

    /// The `CallToolResult` of a failed command: `isError` with the
    /// error (`{ code, message, data? }`) as JSON text.
    #[must_use]
    pub fn error_result(error: &OpError) -> Value {
        let text = serde_json::to_string(error).unwrap_or_else(|_| error.message.clone());
        json!({ "content": [{ "type": "text", "text": text }], "isError": true })
    }
}

fn is_object_schema(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
}

/// `schema` when it describes an object, else wrapped so it does (MCP
/// requires `"type": "object"` input schemas).
fn object_schema(schema: &Value) -> Value {
    if is_object_schema(schema) {
        schema.clone()
    } else {
        json!({ "type": "object", "allOf": [schema] })
    }
}

/// Moves every `pngBase64` string out of `value` into `image` content.
fn extract_pngs(value: &mut Value, images: &mut Vec<Value>) {
    match value {
        Value::Object(map) => {
            if map.get(PNG_FIELD).is_some_and(Value::is_string)
                && let Some(Value::String(data)) = map.remove(PNG_FIELD)
            {
                images.push(json!({ "type": "image", "data": data, "mimeType": "image/png" }));
            }
            for v in map.values_mut() {
                extract_pngs(v, images);
            }
        }
        Value::Array(items) => {
            for v in items {
                extract_pngs(v, images);
            }
        }
        _ => {}
    }
}
