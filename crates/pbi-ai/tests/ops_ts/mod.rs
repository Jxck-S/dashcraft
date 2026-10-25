//! JSON Schema (draft 2020-12, as `schemars` writes it) → TypeScript: the typed client of the
//! desktop app (`app/src/lib/ops.gen.ts`). It lives in `pbi-ai`'s tests because the app runs
//! `pbi_ai::catalog()` (the core catalog plus the `ai.*` commands) and `pbi-ops` cannot depend on
//! `pbi-ai`; `dashcraft-app` checks that its registry is exactly that catalog.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use pbi_ops::Registry;
use serde_json::Value;

/// Named types: name → (canonical schema, declaration).
#[derive(Default)]
struct Types {
    decls: BTreeMap<String, (String, String)>,
}

fn canonical(schema: &Value) -> String {
    let mut s = schema.clone();
    if let Some(o) = s.as_object_mut() {
        for k in ["$schema", "$defs", "title"] {
            o.remove(k);
        }
    }
    s.to_string()
}

fn doc_comment(out: &mut String, indent: &str, text: Option<&str>) {
    let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
        return;
    };
    let text = text.replace("*/", "*\\/");
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() == 1 {
        let _ = writeln!(out, "{indent}/** {} */", lines[0].trim_end());
    } else {
        let _ = writeln!(out, "{indent}/**");
        for l in lines {
            let _ = writeln!(out, "{indent} * {}", l.trim_end());
        }
        let _ = writeln!(out, "{indent} */");
    }
}

fn is_ident(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

fn literal(v: &Value) -> String {
    match v {
        Value::String(s) => format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
        other => other.to_string(),
    }
}

fn union(parts: Vec<String>) -> String {
    let mut seen: Vec<String> = Vec::new();
    for p in parts {
        if !seen.contains(&p) {
            seen.push(p);
        }
    }
    match seen.len() {
        0 => "never".to_owned(),
        _ if seen.iter().any(|p| p == "JsonValue") => "JsonValue".to_owned(),
        _ => seen.join(" | "),
    }
}

/// The TypeScript type of `schema` (`indent`: of the enclosing line).
fn ts(schema: &Value, indent: &str) -> String {
    let Some(o) = schema.as_object() else {
        return if schema == &Value::Bool(false) {
            "never".to_owned()
        } else {
            "JsonValue".to_owned()
        };
    };
    if let Some(r) = o.get("$ref").and_then(Value::as_str) {
        return r.rsplit('/').next().unwrap_or("JsonValue").to_owned();
    }
    if let Some(c) = o.get("const") {
        return literal(c);
    }
    if let Some(e) = o.get("enum").and_then(Value::as_array) {
        return union(e.iter().map(literal).collect());
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(alts) = o.get(key).and_then(Value::as_array) {
            return union(alts.iter().map(|a| ts(a, indent)).collect());
        }
    }
    match o.get("type") {
        Some(Value::String(t)) => ts_of_type(t, o, indent),
        Some(Value::Array(types)) => union(
            types
                .iter()
                .filter_map(Value::as_str)
                .map(|t| ts_of_type(t, o, indent))
                .collect(),
        ),
        _ if o.contains_key("properties") => ts_of_type("object", o, indent),
        _ => "JsonValue".to_owned(),
    }
}

fn ts_of_type(t: &str, o: &serde_json::Map<String, Value>, indent: &str) -> String {
    match t {
        "string" => "string".to_owned(),
        "integer" | "number" => "number".to_owned(),
        "boolean" => "boolean".to_owned(),
        "null" => "null".to_owned(),
        "array" => {
            let item = o
                .get("items")
                .map_or_else(|| "JsonValue".to_owned(), |i| ts(i, indent));
            if item.contains(' ') && !item.starts_with('{') {
                format!("Array<{item}>")
            } else {
                format!("{item}[]")
            }
        }
        "object" => object(o, indent),
        _ => "JsonValue".to_owned(),
    }
}

fn object(o: &serde_json::Map<String, Value>, indent: &str) -> String {
    let props = o.get("properties").and_then(Value::as_object);
    let extra = o.get("additionalProperties").filter(|a| a.is_object());
    match (props, extra) {
        (Some(props), _) if !props.is_empty() => {
            let required: Vec<&str> = o
                .get("required")
                .and_then(Value::as_array)
                .map(|r| r.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let inner = format!("{indent}  ");
            let mut out = String::from("{\n");
            for (key, schema) in props {
                doc_comment(
                    &mut out,
                    &inner,
                    schema.get("description").and_then(Value::as_str),
                );
                let name = if is_ident(key) {
                    key.clone()
                } else {
                    literal(&Value::String(key.clone()))
                };
                let optional = if required.contains(&key.as_str()) {
                    ""
                } else {
                    "?"
                };
                let _ = writeln!(out, "{inner}{name}{optional}: {}", ts(schema, &inner));
            }
            out.push_str(indent);
            out.push('}');
            out
        }
        (_, Some(values)) => format!("{{ [key: string]: {} }}", ts(values, indent)),
        _ => "{ [key: string]: never }".to_owned(),
    }
}

impl Types {
    /// Declares `name` = `schema` (root or `$defs` entry).
    fn add(&mut self, name: &str, schema: &Value) {
        let key = canonical(schema);
        if let Some((existing, _)) = self.decls.get(name) {
            assert!(
                *existing == key,
                "two different JSON Schemas are named {name}: give one of the Rust types \
                 another name (the TypeScript client needs unique type names)"
            );
            return;
        }
        let mut decl = String::new();
        doc_comment(
            &mut decl,
            "",
            schema.get("description").and_then(Value::as_str),
        );
        let body = ts(schema, "");
        let plain_object = schema.get("type") == Some(&Value::String("object".into()))
            && schema.get("anyOf").or(schema.get("oneOf")).is_none();
        if plain_object && body.starts_with("{\n") {
            let _ = writeln!(decl, "export interface {name} {body}");
        } else {
            let _ = writeln!(decl, "export type {name} = {body}");
        }
        self.decls.insert(name.to_owned(), (key, decl));
    }

    /// Declares a root schema and its `$defs`; returns the root's name.
    fn add_root(&mut self, schema: &Value, fallback: &str) -> String {
        if let Some(defs) = schema.get("$defs").and_then(Value::as_object) {
            for (name, def) in defs {
                self.add(name, def);
            }
        }
        let name = schema
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(fallback)
            .to_owned();
        self.add(&name, schema);
        name
    }
}

/// `doc.open` → `docOpen`.
fn camel(id: &str) -> String {
    let mut out = String::new();
    for (i, part) in id.split('.').enumerate() {
        let mut chars = part.chars();
        if let Some(first) = chars.next() {
            if i == 0 {
                out.push(first);
            } else {
                out.extend(first.to_uppercase());
            }
            out.push_str(chars.as_str());
        }
    }
    out
}

fn pascal(id: &str) -> String {
    let c = camel(id);
    let mut chars = c.chars();
    chars.next().map_or_else(String::new, |f| {
        f.to_uppercase().chain(chars).collect::<String>()
    })
}

pub fn generate(registry: &Registry) -> String {
    let mut types = Types::default();
    let mut commands = Vec::new();
    for spec in registry.list() {
        let base = pascal(spec.meta.id);
        let params = types.add_root(&spec.params_schema, &format!("{base}Params"));
        let output = types.add_root(&spec.output_schema, &format!("{base}Output"));
        let optional = spec
            .params_schema
            .get("required")
            .and_then(Value::as_array)
            .is_none_or(Vec::is_empty);
        commands.push((spec.meta, params, output, optional));
    }

    let mut out = String::from(
        "/* eslint-disable */\n\
         // Generated from the app's command registry (pbi-ops + pbi-ai, `pbi_ai::catalog()`) by\n\
         // `npm run gen:ops` (crates/pbi-ai/tests/registry.rs, `ops_typescript_is_up_to_date`).\n\
         // Do not edit: change the Rust command types and regenerate. See docs/COMMANDS.md and\n\
         // docs/AI.md.\n\n\
         /** Any JSON value (`serde_json::Value`). */\n\
         export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue }\n\n",
    );
    for (_, decl) in types.decls.values() {
        out.push_str(decl);
        out.push('\n');
    }

    out.push_str(
        "/** Every command: its parameters and result. */\nexport interface OpsCommands {\n",
    );
    for (meta, params, output, _) in &commands {
        doc_comment(&mut out, "  ", Some(meta.summary));
        let _ = writeln!(
            out,
            "  '{}': {{ params: {params}; output: {output} }}",
            meta.id
        );
    }
    out.push_str("}\n\n");
    out.push_str(
        "export type OpsId = keyof OpsCommands\n\
         export type OpsParams<K extends OpsId> = OpsCommands[K]['params']\n\
         export type OpsOutput<K extends OpsId> = OpsCommands[K]['output']\n\n",
    );

    out.push_str("/** Catalog metadata of every command (`commands.describe` without schemas). */\nexport const OPS_META = {\n");
    for (meta, ..) in &commands {
        let _ = writeln!(
            out,
            "  '{}': {{ category: '{}', mutatesDocument: {}, longRunning: {}, agentSafe: {}, writesFiles: {} }},",
            meta.id,
            meta.category.as_str(),
            meta.mutates_document,
            meta.long_running,
            meta.agent_safe,
            meta.writes_files
        );
    }
    out.push_str("} as const satisfies Record<OpsId, unknown>\n\n");

    out.push_str(
        "/** The outcome of a command: its result, or a failure with the error code (`backend`: the IPC call itself failed). */\n\
         export type OpsResult<T> = { ok: true; value: T } | { ok: false; error: string; kind: OpErrorCode | 'backend'; data?: unknown }\n\n\
         /** Runs command `id` (the app's `ops_execute` IPC command). */\n\
         export type OpsRunner<O> = <K extends OpsId>(id: K, params: OpsParams<K>, options?: O) => Promise<OpsResult<OpsOutput<K>>>\n\n\
         /** Typed wrappers over a runner: `ops.docOpen(params)`. */\n\
         export function makeOps<O = undefined>(run: OpsRunner<O>) {\n  return {\n",
    );
    for (meta, params, _, optional) in &commands {
        doc_comment(&mut out, "    ", Some(meta.summary));
        let default = if *optional { " = {}" } else { "" };
        let _ = writeln!(
            out,
            "    {}: (params: {params}{default}, options?: O) => run('{}', params, options),",
            camel(meta.id),
            meta.id
        );
    }
    out.push_str("  }\n}\n");
    out
}
